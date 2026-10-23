use std::collections::BTreeSet;
use std::time::Instant;

use aya::maps::{HashMap, MapData, PerCpuArray};
use aya::programs::TracePoint;
use aya::{Ebpf, EbpfLoader, include_bytes_aligned};
use hpcd_ebpf::BUCKETS;
use hpcd_probe::SysRoot;

use crate::{BLOCK, Histogram, MeasureError, Measurement};

static PROGRAM: &[u8] = include_bytes_aligned!(concat!(env!("OUT_DIR"), "/hpcd-ebpf"));

const TRACEFS: [&str; 2] = ["/sys/kernel/tracing", "/sys/kernel/debug/tracing"];

fn field_offset(event: &str, field: &str) -> Option<u32> {
    let needle = format!(" {field};");
    TRACEFS.iter().find_map(|root| {
        let format = std::fs::read_to_string(format!("{root}/events/sched/{event}/format")).ok()?;
        let line = format.lines().find(|l| l.contains(&needle))?;
        let rest = &line[line.find("offset:")? + 7..];
        rest[..rest.find(';')?].trim().parse().ok()
    })
}

fn ebpf_err(context: &str) -> impl FnOnce(String) -> MeasureError + '_ {
    move |message| MeasureError::Ebpf {
        context: context.into(),
        message,
    }
}

fn raise_memlock() {
    let unlimited = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &unlimited) };
}

fn load() -> Result<Ebpf, MeasureError> {
    raise_memlock();
    let offsets = [
        (
            "WAKEUP_PID",
            field_offset("sched_wakeup", "pid").unwrap_or(24),
        ),
        (
            "SWITCH_PREV_PID",
            field_offset("sched_switch", "prev_pid").unwrap_or(24),
        ),
        (
            "SWITCH_PREV_STATE",
            field_offset("sched_switch", "prev_state").unwrap_or(32),
        ),
        (
            "SWITCH_NEXT_PID",
            field_offset("sched_switch", "next_pid").unwrap_or(56),
        ),
    ];
    let mut loader = EbpfLoader::new();
    for (name, value) in &offsets {
        loader.override_global(name, value, true);
    }
    let mut ebpf = loader.load(PROGRAM).map_err(|e| {
        ebpf_err("loading eBPF (needs root, or CAP_BPF and CAP_PERFMON)")(e.to_string())
    })?;
    for event in ["sched_wakeup", "sched_wakeup_new", "sched_switch"] {
        let program: &mut TracePoint = ebpf
            .program_mut(event)
            .ok_or_else(|| ebpf_err(event)("program missing from object".into()))?
            .try_into()
            .map_err(|e: aya::programs::ProgramError| ebpf_err(event)(e.to_string()))?;
        program.load().map_err(|e| ebpf_err(event)(e.to_string()))?;
        program
            .attach("sched", event)
            .map_err(|e| ebpf_err(event)(e.to_string()))?;
    }
    Ok(ebpf)
}

fn snapshot(hist: &PerCpuArray<MapData, u64>) -> Result<Histogram, MeasureError> {
    let counts = (0..BUCKETS)
        .map(|i| {
            hist.get(&i, 0)
                .map(|per_cpu| per_cpu.iter().sum())
                .map_err(|e| ebpf_err("reading histogram")(e.to_string()))
        })
        .collect::<Result<Vec<u64>, _>>()?;
    Ok(Histogram::from_counts(counts))
}

fn threads_of(root: &SysRoot, pids: &[u32]) -> BTreeSet<u32> {
    pids.iter()
        .flat_map(|pid| root.list(&format!("/proc/{pid}/task")).unwrap_or_default())
        .filter_map(|tid| tid.parse().ok())
        .collect()
}

pub fn run(
    pids: Vec<u32>,
    blocks: usize,
    mut progress: impl FnMut(usize, usize),
) -> Result<Measurement, MeasureError> {
    let mut ebpf = load()?;
    let take = |ebpf: &mut Ebpf, name: &str| {
        ebpf.take_map(name)
            .ok_or_else(|| ebpf_err(name)("map missing from object".into()))
    };
    let mut targets: HashMap<MapData, u32, u8> = HashMap::try_from(take(&mut ebpf, "TARGETS")?)
        .map_err(|e| ebpf_err("TARGETS")(e.to_string()))?;
    let hist: PerCpuArray<MapData, u64> = PerCpuArray::try_from(take(&mut ebpf, "HIST")?)
        .map_err(|e| ebpf_err("HIST")(e.to_string()))?;

    let root = SysRoot::live();
    let mut seen = BTreeSet::new();
    let mut previous = snapshot(&hist)?;
    let mut result = Vec::with_capacity(blocks);
    let start = Instant::now();
    for i in 0..blocks {
        for tid in threads_of(&root, &pids) {
            if seen.insert(tid) {
                targets
                    .insert(tid, 1, 0)
                    .map_err(|e| ebpf_err("adding target thread")(e.to_string()))?;
            }
        }
        let deadline = start + BLOCK * (i as u32 + 1);
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        let current = snapshot(&hist)?;
        result.push(current.since(&previous));
        previous = current;
        progress(i + 1, blocks);
    }
    drop(ebpf);
    Ok(Measurement {
        pids,
        threads: seen.len(),
        blocks: result,
    })
}
