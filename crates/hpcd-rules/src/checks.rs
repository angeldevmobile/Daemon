use std::collections::BTreeMap;

use hpcd_core::Evidence;
use hpcd_probe::{CpuSet, IdleState, Irq, Observation, Process, Thread};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessRunning {
    names: Vec<String>,
    consequence: Option<String>,
}

pub fn process_running(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let p: ProcessRunning = params(&rule.detect.params)?;
    let found: Vec<&Process> = obs
        .processes
        .iter()
        .filter(|proc| p.names.contains(&proc.comm))
        .collect();
    let Some(first) = found.first() else {
        return Ok(None);
    };
    let mut message = format!("{} is running", first.comm);
    if let Some(c) = &p.consequence {
        message += &format!(" and {c}");
    }
    let evidence = found
        .iter()
        .map(|proc| Evidence {
            source: format!("/proc/{}/comm", proc.pid),
            excerpt: format!("{} (pid {})", proc.comm, proc.pid),
        })
        .collect();
    Ok(Some(Hit { message, evidence }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParams {}

pub fn tuned_profile(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let NoParams {} = params(&rule.detect.params)?;
    let tuned = obs.processes.iter().find(|p| p.comm == "tuned");
    let (Some(tuned), Some(profile)) = (tuned, &obs.tuned_profile) else {
        return Ok(None);
    };
    Ok(Some(Hit {
        message: format!(
            "tuned is running with profile '{profile}'; hpcd will not override settings it manages"
        ),
        evidence: vec![
            Evidence {
                source: "/etc/tuned/active_profile".into(),
                excerpt: profile.clone(),
            },
            Evidence {
                source: format!("/proc/{}/comm", tuned.pid),
                excerpt: format!("tuned (pid {})", tuned.pid),
            },
        ],
    }))
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
    use std::time::Duration;

    use super::*;

    fn rule(check: &str, params: &str) -> Rule {
        toml::from_str(&format!(
            "id = \"t\"\ntitle = \"t\"\ncategory = \"conflicts\"\nseverity = \"warn\"\n\
             platform = \"linux\"\n[detect]\ncheck = \"{check}\"\n{params}\n\
             [explain]\nsummary = \"s\"\n[references]\nlinks = [\"l\"]"
        ))
        .unwrap()
    }

    fn host(processes: &[(u32, &str)], tuned_profile: Option<&str>) -> Observation {
        Observation {
            interval: Duration::from_secs(1),
            pids: vec![],
            threads: vec![],
            irqs: vec![],
            cpus: Default::default(),
            processes: processes
                .iter()
                .map(|(pid, comm)| Process {
                    pid: *pid,
                    comm: comm.to_string(),
                })
                .collect(),
            tuned_profile: tuned_profile.map(String::from),
        }
    }

    #[test]
    fn tuned_needs_the_daemon_and_a_profile() {
        let r = rule("tuned_profile", "");
        let running = host(&[(900, "tuned")], Some("throughput-performance"));
        let hit = tuned_profile(&r, &running).unwrap().unwrap();
        assert!(hit.message.contains("'throughput-performance'"));
        assert!(
            tuned_profile(&r, &host(&[], Some("balanced")))
                .unwrap()
                .is_none()
        );
        assert!(
            tuned_profile(&r, &host(&[(900, "tuned")], None))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn process_running_ignores_other_processes() {
        let r = rule("process_running", "names = [\"irqbalance\"]");
        assert!(
            process_running(&r, &host(&[(1, "systemd")], None))
                .unwrap()
                .is_none()
        );
        let hit = process_running(&r, &host(&[(812, "irqbalance")], None))
            .unwrap()
            .unwrap();
        assert_eq!(hit.message, "irqbalance is running");
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(thousands(8300.4), "8,300");
        assert_eq!(thousands(999.0), "999");
        assert_eq!(thousands(1234567.0), "1,234,567");
    }
}
