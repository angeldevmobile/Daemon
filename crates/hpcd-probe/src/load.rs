use std::collections::BTreeMap;

use crate::SysRoot;
use crate::tasks::{field, parse_stat};

const POWERCAP: &str = "/sys/class/powercap";
const NODE_DIR: &str = "/sys/devices/system/node";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadCounters {
    pub cpu_ticks: BTreeMap<u32, u64>,
    pub faults: BTreeMap<u32, u64>,
    pub switches: BTreeMap<u32, u64>,
    pub rss_kib: u64,
    pub energy_uj: BTreeMap<String, (u64, u64)>,
    pub numa_misses: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Load {
    pub cpu_ticks: u64,
    pub switches: u64,
    pub faults: u64,
    pub rss_kib: u64,
    pub energy_uj: Option<u64>,
    pub numa_misses: Option<u64>,
}

fn delta(now: &BTreeMap<u32, u64>, then: &BTreeMap<u32, u64>) -> u64 {
    now.iter()
        .map(|(id, v)| v.saturating_sub(then.get(id).copied().unwrap_or(0)))
        .sum()
}

impl LoadCounters {
    pub fn since(&self, earlier: &LoadCounters) -> Load {
        let energy_uj = (!self.energy_uj.is_empty()
            && self.energy_uj.keys().eq(earlier.energy_uj.keys()))
        .then(|| {
            self.energy_uj
                .iter()
                .zip(earlier.energy_uj.values())
                .map(|((_, &(now, max)), &(then, _))| {
                    if now >= then {
                        now - then
                    } else {
                        now + max - then
                    }
                })
                .sum()
        });
        Load {
            cpu_ticks: delta(&self.cpu_ticks, &earlier.cpu_ticks),
            switches: delta(&self.switches, &earlier.switches),
            faults: delta(&self.faults, &earlier.faults),
            rss_kib: self.rss_kib,
            energy_uj,
            numa_misses: self
                .numa_misses
                .zip(earlier.numa_misses)
                .map(|(now, then)| now.saturating_sub(then)),
        }
    }
}

fn faults(stat: &str) -> Option<u64> {
    let fields: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    let minor: u64 = fields.get(7)?.parse().ok()?;
    let major: u64 = fields.get(9)?.parse().ok()?;
    Some(minor + major)
}

fn read_energy(root: &SysRoot) -> BTreeMap<String, (u64, u64)> {
    root.list(POWERCAP)
        .unwrap_or_default()
        .into_iter()
        .filter(|name| {
            name.strip_prefix("intel-rapl:")
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|name| {
            let read = |file| {
                root.read(&format!("{POWERCAP}/{name}/{file}"))
                    .ok()?
                    .trim()
                    .parse::<u64>()
                    .ok()
            };
            let value = (read("energy_uj")?, read("max_energy_range_uj")?);
            Some((name, value))
        })
        .collect()
}

fn read_numa_misses(root: &SysRoot) -> Option<u64> {
    let stats: Vec<u64> = root
        .list(NODE_DIR)
        .unwrap_or_default()
        .iter()
        .filter(|name| name.starts_with("node"))
        .filter_map(|name| root.read(&format!("{NODE_DIR}/{name}/numastat")).ok())
        .filter_map(|text| field(&text, "numa_miss", ' ')?.parse().ok())
        .collect();
    (stats.len() > 1).then(|| stats.iter().sum())
}

pub fn read_load(root: &SysRoot, pids: &[u32]) -> LoadCounters {
    let mut load = LoadCounters {
        energy_uj: read_energy(root),
        numa_misses: read_numa_misses(root),
        ..Default::default()
    };
    for &pid in pids {
        if let Ok(stat) = root.read(&format!("/proc/{pid}/stat")) {
            if let Some(s) = parse_stat(&stat) {
                load.cpu_ticks.insert(pid, s.cpu_ticks);
            }
            if let Some(f) = faults(&stat) {
                load.faults.insert(pid, f);
            }
        }
        if let Ok(status) = root.read(&format!("/proc/{pid}/status")) {
            load.rss_kib += field(&status, "VmRSS", ':')
                .and_then(|v| v.trim_end_matches("kB").trim().parse::<u64>().ok())
                .unwrap_or(0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_new_threads_and_ignores_exited_ones() {
        let earlier = LoadCounters {
            cpu_ticks: [(10, 100)].into(),
            switches: [(10, 50), (11, 70)].into(),
            ..Default::default()
        };
        let later = LoadCounters {
            cpu_ticks: [(10, 160)].into(),
            switches: [(10, 80), (12, 5)].into(),
            ..Default::default()
        };
        let load = later.since(&earlier);
        assert_eq!((load.cpu_ticks, load.switches), (60, 35));
        assert_eq!(load.energy_uj, None);
    }

    #[test]
    fn energy_survives_counter_wrap() {
        let at = |a, b| LoadCounters {
            energy_uj: [
                ("intel-rapl:0".into(), (a, 1000)),
                ("intel-rapl:1".into(), (b, 1000)),
            ]
            .into(),
            ..Default::default()
        };
        assert_eq!(at(50, 300).since(&at(900, 100)).energy_uj, Some(150 + 200));
    }

    #[test]
    fn reads_faults_rss_energy_and_numa() {
        let dir = std::env::temp_dir().join(format!("hpcd-load-{}", std::process::id()));
        let write = |path: &str, text: &str| {
            let file = dir.join(path.trim_start_matches('/'));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        write(
            "/proc/7/stat",
            "7 (app) S 1 7 7 0 -1 4194560 120 0 3 0 50 10 0 0 20 0 1 0 1000 100000000 2000 \
             18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 2 0 0 0 0 0",
        );
        write("/proc/7/status", "Name:\tapp\nVmRSS:\t  2048 kB\n");
        write(
            "/proc/7/task/7/status",
            "voluntary_ctxt_switches:\t10\nnonvoluntary_ctxt_switches:\t2\n",
        );
        if cfg!(unix) {
            for (domain, energy, max) in [("0", "5000", "9000"), ("0:0", "1", "9")] {
                let dir = format!("/sys/class/powercap/intel-rapl:{domain}");
                write(&format!("{dir}/energy_uj"), energy);
                write(&format!("{dir}/max_energy_range_uj"), max);
            }
        }
        write(
            "/sys/devices/system/node/node0/numastat",
            "numa_hit 9\nnuma_miss 4\n",
        );
        write(
            "/sys/devices/system/node/node1/numastat",
            "numa_hit 9\nnuma_miss 6\n",
        );

        let load = read_load(&SysRoot::at(&dir), &[7]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(load.cpu_ticks[&7], 60);
        assert_eq!(load.faults[&7], 123);
        assert_eq!(load.switches[&7], 12);
        assert_eq!(load.rss_kib, 2048);
        if cfg!(unix) {
            assert_eq!(
                load.energy_uj,
                [("intel-rapl:0".to_string(), (5000, 9000))].into()
            );
        }
        assert_eq!(load.numa_misses, Some(10));
    }
}
