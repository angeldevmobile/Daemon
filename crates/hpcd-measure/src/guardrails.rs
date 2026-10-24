use hpcd_probe::Load;

use crate::{BLOCK, USER_HZ};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guardrails {
    pub cpu: f64,
    pub power_w: Option<f64>,
    pub rss_kib: u64,
    pub faults: f64,
    pub numa_misses: Option<f64>,
}

pub fn guardrails(load: &[Load]) -> Guardrails {
    let secs = (load.len() as f64 * BLOCK.as_secs_f64()).max(f64::EPSILON);
    let total = |f: fn(&Load) -> u64| load.iter().map(f).sum::<u64>() as f64;
    let rate = |f: fn(&Load) -> Option<u64>| {
        let values: Option<Vec<u64>> = load.iter().map(f).collect();
        values
            .filter(|v| !v.is_empty())
            .map(|v| v.iter().sum::<u64>() as f64 / secs)
    };
    Guardrails {
        cpu: total(|l| l.cpu_ticks) / USER_HZ / secs * 100.0,
        power_w: rate(|l| l.energy_uj).map(|uj_per_s| uj_per_s / 1e6),
        rss_kib: (total(|l| l.rss_kib) / load.len().max(1) as f64).round() as u64,
        faults: total(|l| l.faults) / secs,
        numa_misses: rate(|l| l.numa_misses),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_over_the_measured_blocks() {
        let block = |rss_kib, energy_uj| Load {
            cpu_ticks: 61,
            faults: 120,
            rss_kib,
            energy_uj,
            ..Default::default()
        };
        let g = guardrails(&[
            block(3000, Some(142_000_000)),
            block(3200, Some(142_000_000)),
        ]);
        assert_eq!(g.cpu, 61.0);
        assert_eq!(g.power_w, Some(142.0));
        assert_eq!(g.rss_kib, 3100);
        assert_eq!(g.faults, 120.0);
        assert_eq!(g.numa_misses, None);
    }

    #[test]
    fn power_is_unknown_if_any_block_lacks_it() {
        let g = guardrails(&[
            Load {
                energy_uj: Some(1),
                ..Default::default()
            },
            Load::default(),
        ]);
        assert_eq!(g.power_w, None);
    }
}
