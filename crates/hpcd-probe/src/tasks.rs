use std::collections::BTreeMap;
use std::path::Path;

use hpcd_core::Target;

use crate::{CpuSet, ProbeError, SysRoot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStat {
    pub tid: u32,
    pub comm: String,
    pub cpu_ticks: u64,
    pub processor: u32,
    pub migrations: Option<u64>,
    pub allowed: Option<CpuSet>,
}

pub fn parse_stat(text: &str) -> Option<TaskStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let tid = text[..open].trim().parse().ok()?;
    let comm = text[open + 1..close].to_string();
    let fields: Vec<&str> = text[close + 1..].split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let processor = fields.get(36)?.parse().ok()?;
    Some(TaskStat {
        tid,
        comm,
        cpu_ticks: utime + stime,
        processor,
        migrations: None,
        allowed: None,
    })
}

fn field<'a>(text: &'a str, name: &str, sep: char) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (key, value) = l.split_once(sep)?;
        (key.trim() == name).then(|| value.trim())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub comm: String,
}

pub fn read_processes(root: &SysRoot) -> Vec<Process> {
    root.list("/proc")
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let pid = entry.parse().ok()?;
            let comm = root.read(&format!("/proc/{pid}/comm")).ok()?;
            Some(Process {
                pid,
                comm: comm.trim().to_string(),
            })
        })
        .collect()
}

pub fn read_threads(root: &SysRoot, pid: u32) -> Result<Vec<TaskStat>, ProbeError> {
    let mut threads = Vec::new();
    for tid in root.list(&format!("/proc/{pid}/task"))? {
        let path = format!("/proc/{pid}/task/{tid}/stat");
        let Ok(text) = root.read(&path) else {
            continue;
        };
        let mut stat = parse_stat(&text).ok_or_else(|| ProbeError::Parse {
            path: root.resolve(&path),
            message: "unexpected format".into(),
        })?;
        let dir = format!("/proc/{pid}/task/{tid}");
        if let Ok(sched) = root.read(&format!("{dir}/sched")) {
            stat.migrations = field(&sched, "se.nr_migrations", ':').and_then(|v| v.parse().ok());
        }
        if let Ok(status) = root.read(&format!("{dir}/status")) {
            stat.allowed = field(&status, "Cpus_allowed_list", ':').and_then(CpuSet::parse);
        }
        threads.push(stat);
    }
    Ok(threads)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadCounters {
    pub cpu_ticks: BTreeMap<u32, u64>,
    pub switches: BTreeMap<u32, u64>,
}

impl LoadCounters {
    pub fn since(&self, earlier: &LoadCounters) -> (u64, u64) {
        let delta = |now: &BTreeMap<u32, u64>, then: &BTreeMap<u32, u64>| {
            now.iter()
                .map(|(id, v)| v.saturating_sub(then.get(id).copied().unwrap_or(0)))
                .sum()
        };
        (
            delta(&self.cpu_ticks, &earlier.cpu_ticks),
            delta(&self.switches, &earlier.switches),
        )
    }
}

pub fn read_load(root: &SysRoot, pids: &[u32]) -> LoadCounters {
    let mut load = LoadCounters::default();
    for &pid in pids {
        if let Some(stat) = root
            .read(&format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|t| parse_stat(&t))
        {
            load.cpu_ticks.insert(pid, stat.cpu_ticks);
        }
        for tid in root.list(&format!("/proc/{pid}/task")).unwrap_or_default() {
            let Ok(status) = root.read(&format!("/proc/{pid}/task/{tid}/status")) else {
                continue;
            };
            let count = |name| field(&status, name, ':').and_then(|v| v.parse::<u64>().ok());
            if let (Ok(tid), Some(v), Some(nv)) = (
                tid.parse(),
                count("voluntary_ctxt_switches"),
                count("nonvoluntary_ctxt_switches"),
            ) {
                load.switches.insert(tid, v + nv);
            }
        }
    }
    load
}

pub fn resolve_pids(root: &SysRoot, target: &Target) -> Result<Vec<u32>, ProbeError> {
    let pids = match target {
        Target::Pid(pid) => vec![*pid],
        Target::Cgroup(path) => cgroup_pids(root, &path.to_string_lossy())?,
        Target::Unit(unit) => {
            let cgroup = find_unit_cgroup(root, unit)
                .ok_or_else(|| ProbeError::Target(format!("no cgroup found for unit '{unit}'")))?;
            cgroup_pids(root, &cgroup)?
        }
        Target::Exe(exe) => exe_pids(root, exe)?,
    };
    if pids.is_empty() {
        return Err(ProbeError::Target(format!(
            "no running processes for {target}"
        )));
    }
    Ok(pids)
}

fn cgroup_pids(root: &SysRoot, cgroup: &str) -> Result<Vec<u32>, ProbeError> {
    let cgroup = cgroup.trim_end_matches('/');
    let mut pids: Vec<u32> = root
        .read(&format!("{cgroup}/cgroup.procs"))?
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    for child in root.list(cgroup)? {
        let child = format!("{cgroup}/{child}");
        if root.exists(&format!("{child}/cgroup.procs")) {
            pids.extend(cgroup_pids(root, &child)?);
        }
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

fn find_unit_cgroup(root: &SysRoot, unit: &str) -> Option<String> {
    let direct = format!("/sys/fs/cgroup/system.slice/{unit}");
    if root.exists(&direct) {
        return Some(direct);
    }
    let mut pending = vec!["/sys/fs/cgroup".to_string()];
    for _ in 0..4 {
        let mut next = Vec::new();
        for dir in pending {
            for name in root.list(&dir).ok()? {
                let path = format!("{dir}/{name}");
                if name == unit {
                    return Some(path);
                }
                if root.resolve(&path).is_dir() {
                    next.push(path);
                }
            }
        }
        pending = next;
    }
    None
}

fn exe_pids(root: &SysRoot, exe: &Path) -> Result<Vec<u32>, ProbeError> {
    let mut pids = Vec::new();
    for entry in root.list("/proc")? {
        let Ok(pid) = entry.parse::<u32>() else {
            continue;
        };
        let path = format!("/proc/{pid}/exe");
        let matches = match std::fs::read_link(root.resolve(&path)) {
            Ok(target) => target == exe,
            Err(_) => root.read(&path).is_ok_and(|t| Path::new(t.trim()) == exe),
        };
        if matches {
            root.record(&path, &exe.to_string_lossy());
            pids.push(pid);
        }
    }
    Ok(pids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_spaces_in_comm() {
        let line = "4823 (gw io) S 1 4821 4821 0 -1 4194560 100 0 0 0 1000 200 0 0 20 0 3 0 1000 \
                    100000000 2000 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0";
        let stat = parse_stat(line).unwrap();
        assert_eq!(stat.tid, 4823);
        assert_eq!(stat.comm, "gw io");
        assert_eq!(stat.cpu_ticks, 1200);
        assert_eq!(stat.processor, 3);
    }

    #[test]
    fn load_counts_new_threads_and_ignores_exited_ones() {
        let earlier = LoadCounters {
            cpu_ticks: [(10, 100)].into(),
            switches: [(10, 50), (11, 70)].into(),
        };
        let later = LoadCounters {
            cpu_ticks: [(10, 160)].into(),
            switches: [(10, 80), (12, 5)].into(),
        };
        assert_eq!(later.since(&earlier), (60, 35));
    }
}
