use std::collections::BTreeMap;

use crate::SysRoot;

const CPU_DIR: &str = "/sys/devices/system/cpu";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleState {
    pub index: u32,
    pub name: String,
    pub latency_us: u64,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuPower {
    pub governor: Option<String>,
    pub driver: Option<String>,
    pub idle_states: Vec<IdleState>,
}

pub fn read_cpus(root: &SysRoot) -> BTreeMap<u32, CpuPower> {
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

fn read_cpu(root: &SysRoot, cpu: u32) -> CpuPower {
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
    CpuPower {
        governor: value("cpufreq/scaling_governor"),
        driver: value("cpufreq/scaling_driver"),
        idle_states,
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
