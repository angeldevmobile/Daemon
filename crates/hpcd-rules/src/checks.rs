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
    let thread = threads[0];
    let mut message = format!(
        "IRQs for {} are handled on core {core},\nwhere hot thread {} ({}) runs · {} IRQs/s",
        irq_names(&names),
        thread.tid,
        thread.comm,
        thousands(*total)
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

pub fn cpu_isolation(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let NoParams {} = params(&rule.detect.params)?;
    let hot = obs.hot_cores();
    let Some(isolated) = &obs.isolated else {
        return Ok(None);
    };
    let exposed: CpuSet = hot.iter().filter(|c| !isolated.contains(*c)).collect();
    if exposed.is_empty() {
        return Ok(None);
    }
    let message = if isolated.is_empty() {
        "No isolated cores (requires reboot, proposal only)".to_string()
    } else {
        format!(
            "Hot {} not isolated, isolated: {isolated} (requires reboot, proposal only)",
            cores_phrase(&exposed)
        )
    };
    let shown = if isolated.is_empty() {
        "(empty)".to_string()
    } else {
        isolated.to_string()
    };
    Ok(Some(Hit {
        message,
        evidence: vec![Evidence {
            source: "/sys/devices/system/cpu/isolated".into(),
            excerpt: format!("{shown} · hot {}", cores_phrase(&hot)),
        }],
    }))
}

pub fn smt_siblings(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let NoParams {} = params(&rule.detect.params)?;
    if obs.smt_active != Some(true) {
        return Ok(None);
    }
    let mut shared = Vec::new();
    let mut evidence = Vec::new();
    for core in obs.hot_cores().iter() {
        let Some(siblings) = obs.cpus.get(&core).and_then(|c| c.siblings.as_ref()) else {
            continue;
        };
        let others: CpuSet = siblings.iter().filter(|s| *s != core).collect();
        if others.is_empty() {
            continue;
        }
        let word = if others.iter().count() == 1 {
            "cpu"
        } else {
            "cpus"
        };
        shared.push(format!(
            "hot core {core} shares a core with {word} {others}"
        ));
        evidence.push(Evidence {
            source: format!("/sys/devices/system/cpu/cpu{core}/topology/thread_siblings_list"),
            excerpt: siblings.to_string(),
        });
    }
    if shared.is_empty() {
        return Ok(None);
    }
    evidence.insert(
        0,
        Evidence {
            source: "/sys/devices/system/cpu/smt/active".into(),
            excerpt: "1".into(),
        },
    );
    Ok(Some(Hit {
        message: format!("SMT on · {}", shared.join("; ")),
        evidence,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadMigration {
    target: String,
    min_rate: f64,
}

pub fn thread_migration(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let p: ThreadMigration = params(&rule.detect.params)?;
    hot_threads_target(&p.target)?;
    let secs = obs.interval.as_secs_f64();

    let migrating: Vec<(&Thread, f64)> = obs
        .hot_threads()
        .filter_map(|t| Some((t, t.migrations_per_sec?)))
        .filter(|(t, rate)| {
            *rate >= p.min_rate && t.allowed.as_ref().is_none_or(|a| a.iter().count() > 1)
        })
        .collect();
    let Some((thread, rate)) = migrating.first() else {
        return Ok(None);
    };
    let allowed = thread
        .allowed
        .as_ref()
        .map_or(String::new(), |a| format!(" (allowed cpus {a})"));
    let mut message = format!(
        "Thread {} ({}) migrated {} times in {}s{allowed}",
        thread.tid,
        thread.comm,
        (rate * secs).round(),
        secs.round()
    );
    if migrating.len() > 1 {
        message += &format!(" · {} more threads", migrating.len() - 1);
    }
    let evidence = migrating
        .iter()
        .map(|(t, r)| Evidence {
            source: format!("/proc/{}/task/{}/sched", t.pid, t.tid),
            excerpt: format!("se.nr_migrations +{}", (r * secs).round()),
        })
        .collect();
    Ok(Some(Hit { message, evidence }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmtSiblingBusy {
    target: String,
    min_busy: f64,
}

pub fn smt_sibling_busy(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let p: SmtSiblingBusy = params(&rule.detect.params)?;
    hot_threads_target(&p.target)?;
    let own: CpuSet = obs.threads.iter().map(|t| t.core).collect();

    let mut busy = Vec::new();
    for core in obs.hot_cores().iter() {
        let Some(siblings) = obs.cpus.get(&core).and_then(|c| c.siblings.as_ref()) else {
            continue;
        };
        for sibling in siblings.iter().filter(|s| *s != core && !own.contains(*s)) {
            let load = obs.cpu_busy.get(&sibling).copied().unwrap_or(0.0);
            if load >= p.min_busy {
                busy.push((sibling, core, load));
            }
        }
    }
    let Some((sibling, core, load)) = busy.first() else {
        return Ok(None);
    };
    let mut message = format!(
        "Core {sibling}, SMT sibling of hot core {core}, is {:.0}% busy with other work",
        load * 100.0
    );
    if busy.len() > 1 {
        message += &format!(" · {} more siblings", busy.len() - 1);
    }
    let evidence = busy
        .iter()
        .map(|(s, _, l)| Evidence {
            source: "/proc/stat".into(),
            excerpt: format!(
                "cpu{s} {:.0}% busy over {}s",
                l * 100.0,
                obs.interval.as_secs()
            ),
        })
        .collect();
    Ok(Some(Hit { message, evidence }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NumaRemoteMemory {
    target: String,
    min_remote_share: f64,
}

pub fn numa_remote_memory(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let p: NumaRemoteMemory = params(&rule.detect.params)?;
    hot_threads_target(&p.target)?;
    let total: u64 = obs.memory_pages.values().sum();
    if obs.numa_nodes.len() < 2 || total == 0 {
        return Ok(None);
    }

    for thread in obs.hot_threads() {
        let Some(local) = obs.node_of(thread.core) else {
            continue;
        };
        let Some((&remote, &pages)) = obs
            .memory_pages
            .iter()
            .filter(|(node, _)| **node != local)
            .max_by_key(|(_, pages)| **pages)
        else {
            continue;
        };
        let share = pages as f64 / total as f64;
        if share < p.min_remote_share {
            continue;
        }
        let per_node: Vec<String> = obs
            .memory_pages
            .iter()
            .map(|(n, pages)| format!("N{n}={pages}"))
            .collect();
        return Ok(Some(Hit {
            message: format!(
                "{:.0}% of resident memory on node {remote}, hot thread {} runs on node {local}",
                share * 100.0,
                thread.tid
            ),
            evidence: vec![
                Evidence {
                    source: format!("/proc/{}/numa_maps", thread.pid),
                    excerpt: format!("{} pages", per_node.join(" ")),
                },
                Evidence {
                    source: format!("/sys/devices/system/node/node{local}/cpulist"),
                    excerpt: format!("core {} is on node {local}", thread.core),
                },
            ],
        }));
    }
    Ok(None)
}

pub fn sub_numa_clustering(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let NoParams {} = params(&rule.detect.params)?;
    let nodes = obs.numa_nodes.len();
    let packages = obs.packages();
    if packages == 0 || nodes <= packages {
        return Ok(None);
    }
    Ok(Some(Hit {
        message: format!(
            "On · {nodes} NUMA nodes on {}",
            if packages == 1 {
                "1 socket".to_string()
            } else {
                format!("{packages} sockets")
            }
        ),
        evidence: vec![Evidence {
            source: "/sys/devices/system/node, /sys/devices/system/cpu/cpu*/topology/physical_package_id"
                .into(),
            excerpt: format!("{nodes} nodes · {packages} packages"),
        }],
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmiCount {
    min_per_minute: f64,
}

pub fn smi_count(rule: &Rule, obs: &Observation) -> Result<Option<Hit>, String> {
    let SmiCount { min_per_minute } = params(&rule.detect.params)?;
    let Some(smis) = obs.smis else {
        return Ok(None);
    };
    let secs = obs.interval.as_secs_f64();
    if secs <= 0.0 || smis as f64 / secs * 60.0 < min_per_minute {
        return Ok(None);
    }
    let word = if smis == 1 { "SMI" } else { "SMIs" };
    Ok(Some(Hit {
        message: format!(
            "{smis} {word} in {}s
each SMI freezes every core; the OS cannot see or prevent it",
            obs.interval.as_secs()
        ),
        evidence: vec![Evidence {
            source: format!(
                "/dev/cpu/0/msr (MSR_SMI_COUNT {:#x})",
                hpcd_probe::MSR_SMI_COUNT
            ),
            excerpt: format!("+{smis} in {}s", obs.interval.as_secs()),
        }],
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

fn irq_names(names: &[&str]) -> String {
    fn split(n: &str) -> Option<(&str, &str, u32)> {
        let (device, queue) = n.split_once('-')?;
        let (kind, index) = queue.rsplit_once('-')?;
        Some((device, kind, index.parse().ok()?))
    }
    let parts: Option<Vec<_>> = names.iter().map(|n| split(n)).collect();
    if let Some(parts) = parts.filter(|p| p.len() > 1) {
        let (device, kind, _) = parts[0];
        let mut indexes: Vec<u32> = parts.iter().map(|p| p.2).collect();
        indexes.sort_unstable();
        let same = parts.iter().all(|p| p.0 == device && p.1 == kind);
        let contiguous = indexes.windows(2).all(|w| w[1] == w[0] + 1);
        if same && contiguous {
            return format!(
                "{device} ({kind}-{}..{})",
                indexes[0],
                indexes[indexes.len() - 1]
            );
        }
    }
    match names.len() {
        0..=3 => names.join(", "),
        n => format!("{} +{} more", names[..3].join(", "), n - 3),
    }
}

fn thousands(n: f64) -> String {
    let digits = (n.round() as u64).to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
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
            isolated: None,
            smt_active: None,
            cpu_busy: Default::default(),
            numa_nodes: Default::default(),
            memory_pages: Default::default(),
            smis: None,
            machine: Default::default(),
        }
    }

    #[test]
    fn smi_count_needs_the_counter_and_a_minimum_rate() {
        let r = rule("smi_count", "min_per_minute = 6");
        let mut obs = host(&[], None);
        assert!(smi_count(&r, &obs).unwrap().is_none());
        obs.interval = Duration::from_secs(60);
        obs.smis = Some(5);
        assert!(smi_count(&r, &obs).unwrap().is_none());
        obs.smis = Some(38);
        let hit = smi_count(&r, &obs).unwrap().unwrap();
        assert_eq!(
            hit.message,
            "38 SMIs in 60s
each SMI freezes every core; the OS cannot see or prevent it"
        );
    }

    #[test]
    fn sub_numa_clustering_needs_more_nodes_than_sockets() {
        let r = rule("sub_numa_clustering", "");
        let mut obs = host(&[], None);
        for (cpu, package) in [(0, 0), (1, 0), (2, 1), (3, 1)] {
            obs.cpus.insert(
                cpu,
                hpcd_probe::CpuInfo {
                    package: Some(package),
                    ..Default::default()
                },
            );
        }
        for node in 0..2 {
            obs.numa_nodes.insert(node, CpuSet::default());
        }
        assert!(sub_numa_clustering(&r, &obs).unwrap().is_none());

        for node in 2..4 {
            obs.numa_nodes.insert(node, CpuSet::default());
        }
        let hit = sub_numa_clustering(&r, &obs).unwrap().unwrap();
        assert_eq!(hit.message, "On · 4 NUMA nodes on 2 sockets");
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
    fn groups_irq_queues_of_one_device() {
        let rx = ["eth0-rx-0", "eth0-rx-1", "eth0-rx-2", "eth0-rx-3"];
        assert_eq!(irq_names(&rx), "eth0 (rx-0..3)");
        assert_eq!(irq_names(&["nvme0q0"]), "nvme0q0");
        assert_eq!(
            irq_names(&["eth0-rx-0", "eth0-rx-2", "eth0-tx-1", "i915"]),
            "eth0-rx-0, eth0-rx-2, eth0-tx-1 +1 more"
        );
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(thousands(8300.4), "8,300");
        assert_eq!(thousands(999.0), "999");
        assert_eq!(thousands(1234567.0), "1,234,567");
    }
}
