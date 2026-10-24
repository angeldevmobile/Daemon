use std::collections::BTreeMap;

use crate::{CpuSet, SysRoot};

const CPU_DIR: &str = "/sys/devices/system/cpu";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleState {
    pub index: u32,
    pub name: String,
    pub latency_us: u64,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    pub level: u32,
    pub kind: String,
    pub size: String,
    pub shared: Option<CpuSet>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuInfo {
    pub governor: Option<String>,
    pub driver: Option<String>,
    pub khz: Option<(u64, u64, u64)>,
    pub idle_states: Vec<IdleState>,
    pub siblings: Option<CpuSet>,
    pub package: Option<u32>,
}

pub fn read_cpus(root: &SysRoot) -> BTreeMap<u32, CpuInfo> {
    let Ok(entries) = root.list(CPU_DIR) else {
        return BTreeMap::new();
    };
    entries
        .iter()
        .filter_map(|name| {
            let cpu: u32 = name.strip_prefix("cpu")?.parse().ok()?;
            Some((cpu, read_cpu(root, cpu)))
        })
        .collect()
}

pub fn parse_cpu_times(text: &str) -> BTreeMap<u32, (u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let cpu = fields.next()?.strip_prefix("cpu")?.parse().ok()?;
            let ticks: Vec<u64> = fields.filter_map(|f| f.parse().ok()).collect();
            let total: u64 = ticks.iter().take(8).sum();
            let idle = ticks.get(3)? + ticks.get(4).unwrap_or(&0);
            Some((cpu, (total - idle, total)))
        })
        .collect()
}

pub fn read_caches(root: &SysRoot) -> Vec<Cache> {
    let dir = format!("{CPU_DIR}/cpu0/cache");
    root.list(&dir)
        .unwrap_or_default()
        .iter()
        .filter(|name| name.starts_with("index"))
        .filter_map(|name| {
            let value = |file: &str| {
                root.read(&format!("{dir}/{name}/{file}"))
                    .ok()
                    .map(|s| s.trim().to_string())
            };
            Some(Cache {
                level: value("level")?.parse().ok()?,
                kind: value("type")?,
                size: value("size")?,
                shared: value("shared_cpu_list").and_then(|s| CpuSet::parse(&s)),
            })
        })
        .collect()
}

pub fn read_isolated(root: &SysRoot) -> Option<CpuSet> {
    let text = root.read(&format!("{CPU_DIR}/isolated")).ok()?;
    Some(CpuSet::parse(&text).unwrap_or_default())
}

pub fn read_smt_active(root: &SysRoot) -> Option<bool> {
    let text = root.read(&format!("{CPU_DIR}/smt/active")).ok()?;
    Some(text.trim() == "1")
}

fn read_cpu(root: &SysRoot, cpu: u32) -> CpuInfo {
    let dir = format!("{CPU_DIR}/cpu{cpu}");
    let value = |file: &str| {
        root.read(&format!("{dir}/{file}"))
            .ok()
            .map(|s| s.trim().to_string())
    };
    let idle_states = root
        .list(&format!("{dir}/cpuidle"))
        .unwrap_or_default()
        .iter()
        .filter_map(|state| {
            let index = state.strip_prefix("state")?.parse().ok()?;
            Some(IdleState {
                index,
                name: value(&format!("cpuidle/{state}/name"))?,
                latency_us: value(&format!("cpuidle/{state}/latency"))?.parse().ok()?,
                disabled: value(&format!("cpuidle/{state}/disable")).is_some_and(|d| d == "1"),
            })
        })
        .collect();
    CpuInfo {
        governor: value("cpufreq/scaling_governor"),
        driver: value("cpufreq/scaling_driver"),
        khz: ["scaling_cur_freq", "scaling_min_freq", "scaling_max_freq"]
            .map(|f| value(&format!("cpufreq/{f}")).and_then(|v| v.parse().ok()))
            .into_iter()
            .collect::<Option<Vec<u64>>>()
            .map(|v| (v[0], v[1], v[2])),
        idle_states,
        siblings: value("topology/thread_siblings_list").and_then(|s| CpuSet::parse(&s)),
        package: value("topology/physical_package_id").and_then(|s| s.parse().ok()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn reads_governor_and_idle_states() {
        let root = SysRoot::at(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules/tests/order-gateway/t1"),
        );
        let cpus = read_cpus(&root);
        assert_eq!(cpus.len(), 8);
        let cpu3 = &cpus[&3];
        assert_eq!(cpu3.governor.as_deref(), Some("powersave"));
        assert_eq!(cpu3.driver.as_deref(), Some("intel_pstate"));
        let deepest = cpu3
            .idle_states
            .iter()
            .max_by_key(|s| s.latency_us)
            .unwrap();
        assert_eq!((deepest.name.as_str(), deepest.latency_us), ("C6", 133));
    }

    #[test]
    fn missing_sysfs_gives_no_cpus() {
        assert!(read_cpus(&SysRoot::at("does-not-exist")).is_empty());
    }
}
