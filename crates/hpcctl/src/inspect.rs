use std::collections::{BTreeMap, BTreeSet};

use hpcd_probe::{CpuSet, Observation};

use crate::render::{kib, machine, plural, rate};

const PAGE_KIB: u64 = 4;
const MAX_IRQS: usize = 30;

fn cpus_where(obs: &Observation, cores: Option<&CpuSet>) -> Vec<u32> {
    obs.cpus
        .keys()
        .copied()
        .filter(|c| cores.is_none_or(|set| set.contains(*c)))
        .collect()
}

pub fn topology(obs: &Observation) -> String {
    let smt = match obs.smt_active {
        Some(true) => "SMT on",
        Some(false) => "SMT off",
        None => "SMT unknown",
    };
    let numa = match obs.numa_nodes.len() {
        0 => "NUMA unknown".to_string(),
        n => plural(n, "NUMA node", "NUMA nodes"),
    };
    let mut out: Vec<String> = machine(&obs.machine, obs.packages()).into_iter().collect();
    out.push(format!(
        "{} · {} · {smt} · {numa}",
        plural(obs.cpus.len(), "cpu", "cpus"),
        plural(obs.packages(), "socket", "sockets"),
    ));
    let mut sockets: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (cpu, info) in &obs.cpus {
        if let Some(p) = info.package {
            sockets.entry(p).or_default().push(*cpu);
        }
    }
    for (p, cpus) in sockets {
        out.push(format!("socket {p}: cpus {}", CpuSet::from_iter(cpus)));
    }
    for (node, cpus) in &obs.numa_nodes {
        out.push(format!("NUMA node {node}: cpus {cpus}"));
    }
    let pairs: BTreeSet<String> = obs
        .cpus
        .values()
        .filter_map(|c| c.siblings.as_ref())
        .filter(|s| s.iter().count() > 1)
        .map(|s| s.to_string())
        .collect();
    if !pairs.is_empty() {
        out.push(format!(
            "SMT siblings: {}",
            pairs.into_iter().collect::<Vec<_>>().join(" · ")
        ));
    }
    out.push(match &obs.isolated {
        Some(set) if !set.is_empty() => format!("isolated cpus: {set}"),
        Some(_) => "isolated cpus: none".into(),
        None => "isolated cpus: unknown".into(),
    });
    let cpu0 = obs.cpus.get(&0);
    let core = cpu0
        .and_then(|c| c.siblings.as_ref())
        .map(ToString::to_string)
        .unwrap_or_else(|| "0".into());
    let all = CpuSet::from_iter(obs.cpus.keys().copied()).to_string();
    let socket = CpuSet::from_iter(
        obs.cpus
            .iter()
            .filter(|(_, c)| c.package.is_some() && c.package == cpu0.and_then(|c| c.package))
            .map(|(cpu, _)| *cpu),
    )
    .to_string();
    for c in &obs.caches {
        let scope = match c.shared.as_ref().map(ToString::to_string) {
            None => String::new(),
            Some(s) if s == core => " per core".into(),
            Some(s) if s == all => " shared by all cpus".into(),
            Some(s) if s == socket => " per socket".into(),
            Some(s) => format!(" shared by cpus {s}"),
        };
        out.push(format!("cache L{} {} {}{scope}", c.level, c.kind, c.size));
    }
    out.join("\n")
}

pub fn process(obs: &Observation, subject: &str) -> String {
    let mut out = vec![format!(
        "{subject} · {} · {}",
        plural(obs.pids.len(), "process", "processes"),
        plural(obs.threads.len(), "thread", "threads"),
    )];
    let mut threads: Vec<_> = obs.threads.iter().collect();
    threads.sort_by(|a, b| b.cpu_share.total_cmp(&a.cpu_share));
    for t in threads {
        let mut line = format!(
            "tid {} ({}) · cpu {:.0}% · on core {}",
            t.tid,
            t.comm,
            t.cpu_share * 100.0,
            t.core
        );
        if let Some(allowed) = &t.allowed {
            line += &format!(" · allowed {allowed}");
        }
        if let Some(s) = t.switches_per_sec {
            line += &format!(" · switches {}", rate(s));
        }
        if let Some(m) = t.migrations_per_sec {
            line += &format!(" · migrations {}", rate(m));
        }
        out.push(line);
    }
    out.push("run-queue latency: use the measure tool".into());
    out.join("\n")
}

pub fn numa(obs: &Observation, subject: &str) -> String {
    let mut out = vec![format!(
        "{subject} · {}",
        plural(obs.numa_nodes.len(), "NUMA node", "NUMA nodes")
    )];
    let total: u64 = obs.memory_pages.values().sum();
    if total == 0 {
        out.push("memory per node: unknown".into());
    }
    for (node, pages) in &obs.memory_pages {
        out.push(format!(
            "memory on node {node}: {} ({:.0}%)",
            kib(pages * PAGE_KIB),
            *pages as f64 / total as f64 * 100.0
        ));
    }
    let mut running: BTreeMap<Option<u32>, Vec<String>> = BTreeMap::new();
    for t in obs.hot_threads() {
        running
            .entry(obs.node_of(t.core))
            .or_default()
            .push(format!("{} ({}) on core {}", t.tid, t.comm, t.core));
    }
    for (node, threads) in running {
        let node = node.map_or("unknown node".into(), |n| format!("node {n}"));
        out.push(format!("hot threads on {node}: {}", threads.join(", ")));
    }
    out.join("\n")
}

pub fn irqs(obs: &Observation, cores: Option<&CpuSet>) -> String {
    let cpus = cpus_where(obs, cores);
    let mut lines: Vec<(f64, String)> = obs
        .irqs
        .iter()
        .filter_map(|irq| {
            let total: f64 = cpus.iter().map(|c| irq.rate_on(*c)).sum();
            (total > 0.0).then_some((irq, total))
        })
        .map(|(irq, total)| {
            let on: Vec<String> = cpus
                .iter()
                .filter(|c| irq.rate_on(**c) > 0.0)
                .map(|c| format!("core {c} {}", rate(irq.rate_on(*c))))
                .collect();
            let name = if irq.number().is_some() {
                &irq.name
            } else {
                &irq.description
            };
            let affinity = irq
                .affinity
                .as_ref()
                .map_or(String::new(), |a| format!(" · affinity {a}"));
            (
                total,
                format!("irq {} {name} → {}{affinity}", irq.label, on.join(", ")),
            )
        })
        .collect();
    lines.sort_by(|a, b| b.0.total_cmp(&a.0));
    let scope = cores.map_or("all cores".into(), |c| format!("cores {c}"));
    let mut out = vec![format!(
        "{scope} · {} with activity over {}s",
        plural(lines.len(), "IRQ", "IRQs"),
        obs.interval.as_secs()
    )];
    let hidden = lines.len().saturating_sub(MAX_IRQS);
    out.extend(lines.into_iter().take(MAX_IRQS).map(|(_, l)| l));
    if hidden > 0 {
        out.push(format!("… {hidden} more"));
    }
    out.join("\n")
}

pub fn power(obs: &Observation, cores: Option<&CpuSet>) -> String {
    let mut groups: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for cpu in cpus_where(obs, cores) {
        let info = &obs.cpus[&cpu];
        let mut parts = vec![match (&info.governor, &info.driver) {
            (Some(g), Some(d)) => format!("governor {g} ({d})"),
            (Some(g), None) => format!("governor {g}"),
            _ => "no cpufreq".into(),
        }];
        if let Some((cur, min, max)) = info.khz {
            let mhz = |k: u64| k / 1000;
            parts.push(format!(
                "{} MHz (range {}–{})",
                mhz(cur),
                mhz(min),
                mhz(max)
            ));
        }
        let states: Vec<String> = info
            .idle_states
            .iter()
            .filter(|s| !s.disabled)
            .map(|s| format!("{} {}µs", s.name, s.latency_us))
            .collect();
        parts.push(if states.is_empty() {
            "no cpuidle states".into()
        } else {
            format!("C-states {}", states.join(", "))
        });
        groups.entry(parts.join(" · ")).or_default().push(cpu);
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|(_, cpus)| cpus[0]);
    groups
        .into_iter()
        .map(|(state, cpus)| format!("cpus {}: {state}", CpuSet::from_iter(cpus)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use hpcd_core::Target;
    use hpcd_probe::Source;

    use super::*;

    fn fixture() -> Observation {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules/tests/order-gateway");
        hpcd_probe::observe(
            &Source::snapshot(dir).unwrap(),
            &Target::Unit("order-gateway.service".into()),
        )
        .unwrap()
    }

    #[test]
    fn describes_the_fixture() {
        let obs = fixture();
        let topology = topology(&obs);
        assert!(
            topology.starts_with(
                "Dell PowerEdge R650 · BIOS 1.9.2 · 2 × Xeon Gold 6338\n\
                 8 cpus · 2 sockets · SMT on · 2 NUMA nodes"
            ),
            "{topology}"
        );
        assert!(topology.contains("SMT siblings: "), "{topology}");

        let irqs = irqs(&obs, CpuSet::parse("3").as_ref());
        assert!(irqs.starts_with("cores 3 · "), "{irqs}");
        assert!(irqs.contains("eth0"), "{irqs}");

        let power = power(&obs, CpuSet::parse("2-5").as_ref());
        assert!(
            power.contains("governor powersave (intel_pstate)"),
            "{power}"
        );

        let process = process(&obs, "order-gateway.service");
        assert!(
            process.contains("(gw-io) · cpu 95% · on core 3"),
            "{process}"
        );

        let numa = numa(&obs, "order-gateway.service");
        assert!(numa.contains("memory on node"), "{numa}");
    }
}
