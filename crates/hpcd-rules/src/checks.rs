use std::collections::BTreeMap;

use hpcd_core::Evidence;
use hpcd_probe::{CpuSet, IdleState, Irq, Observation, Thread};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::Rule;

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub message: String,
    pub evidence: Vec<Evidence>,
}

fn params<T: DeserializeOwned>(table: &toml::Table) -> Result<T, String> {
    toml::Value::Table(table.clone())
        .try_into()
        .map_err(|e: toml::de::Error| e.message().to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IrqAffinityOverlaps {
    target: String,
    min_irq_rate: f64,
    #[serde(default)]
    exclude: Vec<String>,
}

pub fn irq_affinity_overlaps(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let p: IrqAffinityOverlaps = params(&rule.detect.params)?;
    hot_threads_target(&p.target)?;

    let mut by_core: BTreeMap<u32, Vec<&Thread>> = BTreeMap::new();
    for t in obs.hot_threads() {
        by_core.entry(t.core).or_default().push(t);
    }

    let mut hits = Vec::new();
    for (core, threads) in by_core {
        let irqs: Vec<&Irq> = obs
            .irqs
            .iter()
            .filter(|i| i.number().is_some() && i.rate_on(core) > 0.0)
            .filter(|i| !p.exclude.iter().any(|x| *x == i.label || *x == i.name))
            .collect();
        let total: f64 = irqs.iter().map(|i| i.rate_on(core)).sum();
        if total >= p.min_irq_rate {
            hits.push((core, threads, irqs, total));
        }
    }
    let Some((core, threads, irqs, total)) = hits.first() else {
        return Ok(None);
    };

    let names: Vec<&str> = irqs.iter().map(|i| i.name.as_str()).collect();
    let shown = match names.len() {
        0..=3 => names.join(", "),
        n => format!("{} +{} more", names[..3].join(", "), n - 3),
    };
    let thread = threads[0];
    let mut message = format!(
        "IRQs {shown} are handled on core {core} at {}/s,\nwhere hot thread {} ({}) runs",
        thousands(*total),
        thread.tid,
        thread.comm
    );
    if hits.len() > 1 {
        message += &format!(" · {} more cores affected", hits.len() - 1);
    }

    let evidence = irqs
        .iter()
        .map(|i| Evidence {
            source: "/proc/interrupts".into(),
            excerpt: format!(
                "IRQ {} {} {}/s on core {core} · affinity {}",
                i.label,
                i.name,
                thousands(i.rate_on(*core)),
                i.affinity.as_ref().map_or("?".into(), |a| a.to_string()),
            ),
        })
        .collect();
    Ok(Some(Hit { message, evidence }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CpuidleDeepStates {
    target: String,
    max_exit_latency_us: u64,
}

pub fn cpuidle_deep_states(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let p: CpuidleDeepStates = params(&rule.detect.params)?;
    hot_threads_target(&p.target)?;

    let mut deepest: Option<&IdleState> = None;
    let mut cores = Vec::new();
    let mut evidence = Vec::new();
    for core in obs.hot_cores().iter() {
        let Some(cpu) = obs.cpus.get(&core) else {
            continue;
        };
        let deep: Vec<&IdleState> = cpu
            .idle_states
            .iter()
            .filter(|s| !s.disabled && s.latency_us > p.max_exit_latency_us)
            .collect();
        if deep.is_empty() {
            continue;
        }
        cores.push(core);
        for s in &deep {
            evidence.push(Evidence {
                source: format!("/sys/devices/system/cpu/cpu{core}/cpuidle/state{}", s.index),
                excerpt: format!("{} enabled · exit latency {}µs", s.name, s.latency_us),
            });
        }
        let core_deepest = deep.into_iter().max_by_key(|s| s.latency_us);
        if core_deepest.map(|s| s.latency_us) > deepest.map(|s| s.latency_us) {
            deepest = core_deepest;
        }
    }
    let Some(state) = deepest else {
        return Ok(None);
    };
    let message = format!(
        "{} enabled on {} (exit latency {}µs)",
        state.name,
        cores_phrase(&cores.into_iter().collect()),
        state.latency_us
    );
    Ok(Some(Hit { message, evidence }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CpufreqGovernor {
    target: String,
    governors: Vec<String>,
}

pub fn cpufreq_governor(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let p: CpufreqGovernor = params(&rule.detect.params)?;
    hot_threads_target(&p.target)?;

    let mut by_governor: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    let mut evidence = Vec::new();
    for core in obs.hot_cores().iter() {
        let Some(cpu) = obs.cpus.get(&core) else {
            continue;
        };
        let Some(governor) = cpu.governor.as_deref() else {
            continue;
        };
        if p.governors.iter().any(|g| g == governor) {
            by_governor.entry(governor).or_default().push(core);
            evidence.push(Evidence {
                source: format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_governor"),
                excerpt: format!(
                    "{governor} · driver {}",
                    cpu.driver.as_deref().unwrap_or("?")
                ),
            });
        }
    }
    if by_governor.is_empty() {
        return Ok(None);
    }
    let message = by_governor
        .into_iter()
        .map(|(g, cores)| {
            format!(
                "Governor '{g}' on {}",
                cores_phrase(&cores.into_iter().collect())
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Ok(Some(Hit { message, evidence }))
}

fn hot_threads_target(target: &str) -> Result<(), String> {
    if target == "hot_threads" {
        Ok(())
    } else {
        Err(format!(
            "unsupported target '{target}', expected 'hot_threads'"
        ))
    }
}

fn cores_phrase(cores: &CpuSet) -> String {
    let single = cores.iter().count() == 1;
    format!("{} {cores}", if single { "core" } else { "cores" })
}

fn thousands(n: f64) -> String {
    let digits = (n.round() as u64).to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::thousands;

    #[test]
    fn groups_thousands() {
        assert_eq!(thousands(8300.4), "8,300");
        assert_eq!(thousands(999.0), "999");
        assert_eq!(thousands(1234567.0), "1,234,567");
    }
}
