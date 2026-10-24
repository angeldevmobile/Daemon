use anstyle::Style;
use hpcd_core::Finding;
use std::time::Duration;

use hpcd_measure::{
    Drift, Estimate, Guardrails, Overhead, Signal, Window, format_limit, format_ns, format_range,
};
use hpcd_probe::{Capture, Machine};
use hpcd_rules::{Rule, registry};

use crate::style::{self, paint, severity_badge};

const INDENT: &str = "   ";
const ID_WIDTH: usize = 22;

pub fn explain(rule: &Rule) -> String {
    let mut out = vec![format!(
        "{} · {}",
        paint(style::ID.bold(), &rule.id),
        rule.title
    )];

    let mut meta = format!(
        "{}{}{}{}",
        paint(style::DIM, "category "),
        rule.category.dir_name(),
        paint(style::DIM, "  ·  severity "),
        severity_badge(rule.severity),
    );
    if let Some(fix) = &rule.fix {
        let (reboot, reboot_style) = if fix.reboot {
            ("yes", style::WARN)
        } else {
            ("no", style::OK)
        };
        meta += &format!(
            "{}{}{}{}",
            paint(style::DIM, "  ·  risk "),
            paint(style::risk(fix.risk), fix.risk),
            paint(style::DIM, "  ·  reboot "),
            paint(reboot_style, reboot),
        );
    }
    out.push(meta);

    section(
        &mut out,
        "WHY",
        rule.explain.summary.trim().lines().map(String::from),
    );

    if !rule.explain.evidence.is_empty() {
        let sources = format!("sources: {}", rule.explain.evidence.join(", "));
        section(&mut out, "EVIDENCE", [paint(style::DIM, sources)]);
    }

    let fix_lines = match &rule.fix {
        Some(fix) => {
            let mut action = paint(style::ID, &fix.action);
            if let Some(to) = fix.params.get("to").and_then(|v| v.as_str()) {
                action += &format!(" → {}", to.replace('_', " "));
            }
            let revert = registry::action(&fix.action).map_or("", |a| a.revert);
            vec![action, format!("{}  {revert}", paint(style::DIM, "revert"))]
        }
        None => vec![paint(style::DIM, "detect only · no runtime fix")],
    };
    section(&mut out, "FIX", fix_lines);

    if let Some(fix) = rule.fix.as_ref().filter(|f| !f.conflicts.is_empty()) {
        let lines = fix
            .conflicts
            .iter()
            .map(|(tool, advice)| format!("{}{tool}: {advice}", paint(style::WARN, "⚠ ")));
        section(&mut out, "CONFLICTS", lines);
    }

    if !rule.propose.is_empty() {
        let width = rule.propose.keys().map(String::len).max().unwrap_or(0);
        let lines = rule.propose.iter().flat_map(|(vendor, steps)| {
            steps
                .iter()
                .map(move |s| format!("{}  {s}", paint(style::DIM, format!("{vendor:<width$}"))))
        });
        section(&mut out, "PROPOSE", lines);
    }

    section(
        &mut out,
        "REFERENCES",
        rule.references.links.iter().map(|l| paint(style::LINK, l)),
    );
    out.join("\n")
}

pub enum Footer<'a> {
    Fix(&'a str),
    Text(String),
}

pub fn audit(
    title: &str,
    subject: &str,
    qualifier: Option<&str>,
    details: &[String],
    findings: &[Finding],
    footer: Footer,
) -> String {
    let mut findings: Vec<&Finding> = findings.iter().collect();
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));

    let mut header = format!(
        "{}{}{subject}",
        paint(style::ACCENT, title),
        paint(style::DIM, " · ")
    );
    if let Some(q) = qualifier {
        header += &paint(style::DIM, format!(" {q}"));
    }
    let mut rest: Vec<String> = details.to_vec();
    rest.push(plural(findings.len(), "rule", "rules") + " matched");
    header += &paint(style::DIM, format!(" · {}", rest.join(" · ")));

    let mut out = vec![header, String::new()];
    if findings.is_empty() {
        out.push(paint(style::OK_BOLD, "✔ no issues found"));
        return out.join("\n");
    }
    let continuation = " ".repeat(1 + 6 + 2 + ID_WIDTH);
    for f in &findings {
        let mut lines = f.message.lines();
        out.push(format!(
            " {}  {}{}",
            severity_badge(f.severity),
            paint(style::ID, format!("{:<w$} ", f.rule_id, w = ID_WIDTH - 1)),
            lines.next().unwrap_or_default(),
        ));
        out.extend(lines.map(|l| format!("{continuation}{l}")));
    }

    out.push(String::new());
    let fix_command = match footer {
        Footer::Text(text) => {
            out.push(text);
            return out.join(
                "
",
            );
        }
        Footer::Fix(command) => command,
    };
    let fixable = findings.iter().filter(|f| f.fixable_at_runtime).count();
    let sep = paint(style::DIM, " · ");
    let mut footer = format!(
        "{}{sep}{}{}",
        paint(style::OK_BOLD, format!("{fixable} fixable at runtime")),
        paint(style::ID, "hpcctl explain <rule>"),
        paint(style::DIM, " for details"),
    );
    if fixable > 0 {
        footer += &format!("{sep}{}", paint(style::ID, fix_command));
    }
    out.push(footer);
    out.join("\n")
}

pub fn machine(machine: &Machine, packages: usize) -> Option<String> {
    let mut parts = Vec::new();
    let name = [
        machine.vendor.as_deref().map(vendor_name),
        machine.product.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    if !name.is_empty() {
        parts.push(name);
    }
    if let Some(bios) = &machine.bios {
        parts.push(format!("BIOS {bios}"));
    }
    if let Some(model) = &machine.cpu_model {
        parts.push(if packages > 1 {
            format!("{packages} × {model}")
        } else {
            model.clone()
        });
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn vendor_name(vendor: &str) -> String {
    let lower = vendor.to_lowercase();
    let known = [
        ("dell", "Dell"),
        ("hewlett", "HPE"),
        ("hpe", "HPE"),
        ("lenovo", "Lenovo"),
        ("supermicro", "Supermicro"),
    ];
    known
        .iter()
        .find(|(key, _)| lower.contains(key))
        .map_or(vendor.to_string(), |(_, name)| name.to_string())
}

pub fn firmware_proposal(machine: &Machine, rules: &[&Rule]) -> String {
    let vendor = machine.vendor.as_deref().map(vendor_name);
    let key = vendor.as_deref().map(str::to_lowercase);
    let has_vendor_steps = |r: &&Rule| key.as_ref().is_some_and(|k| r.propose.contains_key(k));
    let (label, place) = match (key.as_deref(), rules.iter().any(has_vendor_steps)) {
        (Some("dell"), true) => ("Dell", "System Setup (F2) or iDRAC → BIOS Settings"),
        (Some("hpe"), true) => ("HPE", "System Utilities (F9) or iLO → BIOS"),
        (Some("lenovo"), true) => ("Lenovo", "System Setup (F1) or XClarity Controller"),
        (Some("supermicro"), true) => ("Supermicro", "BIOS Setup (Del) or BMC"),
        _ => ("generic", "the firmware setup utility"),
    };

    let mut out = vec![format!(
        "{}{}",
        paint(style::ACCENT, "proposal"),
        paint(style::DIM, format!(" · {label} · {place}"))
    )];
    let steps = rules.iter().flat_map(|r| {
        key.as_ref()
            .and_then(|k| r.propose.get(k))
            .or_else(|| r.propose.get("generic"))
            .into_iter()
            .flatten()
    });
    for (i, step) in steps.enumerate() {
        let step = match step.rsplit_once(" = ") {
            Some((setting, value)) => format!("{setting} = {}", paint(style::ID.bold(), value)),
            None => step.clone(),
        };
        out.push(format!(
            "{}  {step}",
            paint(style::DIM, format!("{}.", i + 1))
        ));
    }
    out.push(String::new());
    out.push(paint(
        style::DIM,
        "hpcd never writes BIOS settings. After the reboot, re-run this audit to verify.",
    ));
    out.join("\n")
}

pub fn capture(
    subject: &str,
    qualifier: Option<&str>,
    captured: &Capture,
    audit_command: Option<&str>,
) -> String {
    let mut header = format!(
        "{}{}{subject}",
        paint(style::ACCENT, "hpcd capture"),
        paint(style::DIM, " · ")
    );
    if let Some(q) = qualifier {
        header += &paint(style::DIM, format!(" {q}"));
    }
    header += &paint(
        style::DIM,
        format!(" · 2 samples {}s apart", captured.interval.as_secs()),
    );
    let mut out = vec![header, String::new()];

    if audit_command.is_none() {
        let width = captured.files().map(|(_, p, _)| p.len()).max().unwrap_or(0);
        for (t, path, size) in captured.files() {
            out.push(format!(
                "{INDENT}{}  {path:<width$}  {}",
                paint(style::DIM, t),
                paint(style::DIM, format!("{:>9}", bytes(size)))
            ));
        }
        out.push(String::new());
    }

    for (t, files) in ["t0", "t1"].into_iter().zip(&captured.samples) {
        let size = files.values().map(String::len).sum();
        let word = if files.len() == 1 { "file" } else { "files" };
        out.push(format!(
            "{INDENT}{}  {:>4} {word:<5}  {:>9}",
            paint(style::DIM, t),
            files.len(),
            bytes(size)
        ));
    }
    let mut roots: Vec<&str> = captured
        .files()
        .map(|(_, path, _)| {
            if path.starts_with("/etc/") {
                path
            } else {
                path.split('/')
                    .nth(1)
                    .map_or(path, |r| &path[..r.len() + 1])
            }
        })
        .collect();
    roots.sort_unstable();
    roots.dedup();
    out.push(paint(
        style::DIM,
        format!(
            "{INDENT}from {} · no memory or file contents",
            roots.join(", ")
        ),
    ));
    out.push(String::new());

    out.push(match audit_command {
        Some(cmd) => format!(
            "{}{}{}",
            paint(style::OK_BOLD, "✔ written"),
            paint(style::DIM, " · "),
            paint(style::ID, cmd)
        ),
        None => paint(style::DIM, "nothing written (--show)"),
    });
    out.join("\n")
}

fn bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else {
        format!("{:.1} KiB", n as f64 / 1024.0)
    }
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped} {}", if n == 1 { one } else { many })
}

pub struct LatencyRow {
    pub label: String,
    pub estimate: Estimate,
    pub check: Option<(u64, bool)>,
}

fn measure_header(subject: &str, qualifier: Option<&str>, details: &[String]) -> String {
    let mut header = format!(
        "{}{}{subject}",
        paint(style::ACCENT, "hpcd measure"),
        paint(style::DIM, " · ")
    );
    if let Some(q) = qualifier {
        header += &paint(style::DIM, format!(" {q}"));
    }
    header + &paint(style::DIM, format!(" · {}", details.join(" · ")))
}

pub fn measure(
    subject: &str,
    qualifier: Option<&str>,
    details: &[String],
    summary: &str,
    guardrails: Option<&Guardrails>,
    overhead: Option<&Overhead>,
    rows: &[LatencyRow],
) -> String {
    let mut out = vec![
        measure_header(subject, qualifier, details),
        paint(style::DIM, format!("{INDENT}{summary}")),
    ];
    if let Some(g) = guardrails {
        out.push(paint(style::DIM, format!("{INDENT}{}", guardrail_line(g))));
    }
    if let Some(o) = overhead {
        out.push(paint(style::DIM, format!("{INDENT}{}", overhead_line(o))));
    }
    out.push(String::new());
    for row in rows {
        let e = row.estimate;
        let range = format_range(e.low, e.high, e.value);
        let width = if row.check.is_some() { 14 } else { 0 };
        let mut line = format!(
            "  {:<5}{}  {}",
            row.label,
            paint(Style::new().bold(), format!("{:>9}", format_ns(e.value))),
            paint(style::DIM, format!("{range:<width$}")),
        );
        if let Some((limit, ok)) = row.check {
            line += &paint(
                style::DIM,
                format!("{:<16}", format!("budget {}", format_limit(limit))),
            );
            line += &if ok {
                paint(style::OK_BOLD, "✔ pass")
            } else {
                paint(style::BAD.bold(), "✘ fail")
            };
        }
        out.push(line.trim_end().to_string());
    }
    let checks: Vec<bool> = rows
        .iter()
        .filter_map(|r| r.check.map(|(_, ok)| ok))
        .collect();
    if !checks.is_empty() {
        out.push(String::new());
        out.push(if checks.iter().all(|ok| *ok) {
            paint(style::OK_BOLD, "✔ all latency budgets met")
        } else {
            paint(style::BAD.bold(), "✘ tail latency budget exceeded")
        });
    }
    out.join(
        "
",
    )
}

pub fn unstable(
    subject: &str,
    qualifier: Option<&str>,
    details: &[String],
    windows: &[Window],
    drift: &[Drift],
    waited: Option<Duration>,
) -> String {
    let mut out = vec![measure_header(subject, qualifier, details), String::new()];
    let first = windows.iter().find_map(|w| w.p99);
    for (i, w) in windows.iter().enumerate() {
        let p99 = w.p99.map_or("—".to_string(), format_ns);
        let p99_style = match (w.p99, first) {
            (Some(v), Some(f)) if v >= f * 2 => style::BAD.bold(),
            (Some(v), Some(f)) if v * 2 >= f * 3 => style::CAUTION.bold(),
            _ => Style::new().bold(),
        };
        out.push(format!(
            "{INDENT}window {}/{}   p99 {}{}",
            i + 1,
            windows.len(),
            paint(p99_style, format!("{p99:>7}")),
            paint(
                style::DIM,
                format!(
                    "   cpu {:>4}   wakeups {:>7}   switches {:>7}",
                    format!("{:.0}%", w.cpu),
                    rate(w.wakeups),
                    rate(w.switches)
                )
            ),
        ));
    }

    let changes: Vec<String> = drift
        .iter()
        .map(|d| match d.signal {
            Signal::Cpu => format!("cpu {:+.0} pts", d.to - d.from),
            Signal::Wakeups | Signal::Switches => {
                let name = if d.signal == Signal::Wakeups {
                    "wakeups"
                } else {
                    "switches"
                };
                format!("{name} {:+.0}%", (d.to - d.from) / d.from.max(1.0) * 100.0)
            }
        })
        .collect();
    let changes = match changes.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    let mut why = format!("{INDENT}{changes} across windows");
    if let Some(wait) = waited {
        why += &format!(" · waited {}", elapsed(wait));
    }
    out.push(String::new());
    out.push(format!(
        "{}{}",
        paint(style::BAD.bold(), "✘ load is not stable"),
        paint(style::DIM, why)
    ));
    out.push("  A percentile now would mix different workloads, not describe one.".into());
    out.push("  No verdict was given.".into());
    if waited.is_none() {
        out.push(String::new());
        out.push(paint(style::DIM, "  try:"));
        out.push(format!(
            "    · wait for steady load, then measure    {}",
            paint(style::ACCENT, "hpcctl measure … --wait-stable 10m")
        ));
    }
    out.join("\n")
}

fn guardrail_line(g: &Guardrails) -> String {
    let mut parts = vec![format!("cpu {:.0}%", g.cpu)];
    if let Some(w) = g.power_w {
        parts.push(format!("power {w:.0}W"));
    }
    parts.push(format!("rss {}", kib(g.rss_kib)));
    parts.push(format!("faults {}", rate(g.faults)));
    if let Some(m) = g.numa_misses {
        parts.push(format!("numa misses {}", rate(m)));
    }
    parts.join(" · ")
}

fn overhead_line(o: &Overhead) -> String {
    let cpu = o.cpu();
    let cpu = if cpu < 10.0 {
        format!("{cpu:.1}%")
    } else {
        format!("{cpu:.0}%")
    };
    let ebpf = match o.per_event() {
        Some(d) => format!(" · eBPF {} per event", format_ns(d.as_nanos() as u64)),
        None if o.ebpf.is_none() => " (eBPF not counted)".into(),
        None => String::new(),
    };
    format!(
        "hpcd itself {cpu} cpu during measurement{ebpf} · {} rss",
        kib(o.rss_kib)
    )
}

pub fn kib(n: u64) -> String {
    match n {
        0..1024 => format!("{n} KiB"),
        1024..1_048_576 => format!("{:.0} MiB", n as f64 / 1024.0),
        _ => format!("{:.1} GiB", n as f64 / 1_048_576.0),
    }
}

pub fn rate(per_sec: f64) -> String {
    if per_sec < 1000.0 {
        format!("{per_sec:.0}/s")
    } else if per_sec < 1e6 {
        format!("{:.1}k/s", per_sec / 1e3)
    } else {
        format!("{:.1}M/s", per_sec / 1e6)
    }
}

pub fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, 0, _) => format!("{h}h"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

fn section(out: &mut Vec<String>, name: &str, lines: impl IntoIterator<Item = String>) {
    out.push(String::new());
    out.push(paint(style::HEADING, name));
    out.extend(lines.into_iter().map(|l| format!("{INDENT}{l}")));
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use hpcd_core::Severity;

    use super::*;

    fn plain(s: &str) -> String {
        anstream::adapter::strip_str(s).to_string()
    }

    fn finding(rule_id: &str, severity: Severity, message: &str, fixable: bool) -> Finding {
        Finding {
            rule_id: rule_id.into(),
            severity,
            message: message.into(),
            evidence: vec![],
            fixable_at_runtime: fixable,
        }
    }

    #[test]
    fn measure_matches_design() {
        let row = |label: &str, value, low, high, limit, ok| LatencyRow {
            label: label.into(),
            estimate: Estimate { value, low, high },
            check: Some((limit, ok)),
        };
        let rows = [
            row("p99", 182_000, 176_000, 189_000, 200_000, true),
            row("p99.9", 1_310_000, 1_180_000, 1_440_000, 1_000_000, false),
        ];
        let details = [
            "run-queue latency".to_string(),
            "eBPF".into(),
            "5 × 20s".into(),
        ];
        let text = plain(&measure(
            "api.service",
            Some("(pid 4821)"),
            &details,
            "12,431 wakeups · 9 threads",
            Some(&Guardrails {
                cpu: 61.0,
                power_w: Some(142.4),
                rss_kib: 3_250_000,
                faults: 120.0,
                numa_misses: None,
            }),
            Some(&Overhead {
                elapsed: Duration::from_secs(100),
                cpu_ticks: 30,
                rss_kib: 14_336,
                ebpf: Some(hpcd_measure::EbpfCost {
                    run_time: Duration::from_millis(400),
                    runs: 3_300_000,
                }),
            }),
            &rows,
        ));
        let expected = "hpcd measure · api.service (pid 4821) · run-queue latency · eBPF · 5 × 20s
   12,431 wakeups · 9 threads
   cpu 61% · power 142W · rss 3.1 GiB · faults 120/s
   hpcd itself 0.7% cpu during measurement · eBPF 121ns per event · 14 MiB rss

  p99      182µs  [176–189]     budget 200µs    ✔ pass
  p99.9   1.31ms  [1.18–1.44]   budget 1ms      ✘ fail

✘ tail latency budget exceeded";
        assert_eq!(text, expected);
        assert_eq!(plural(12_431, "wakeup", "wakeups"), "12,431 wakeups");
    }

    #[test]
    fn unstable_load_matches_design() {
        let window = |p99, cpu, wakeups| Window {
            p99: Some(p99),
            cpu,
            wakeups,
            switches: 13_000.0,
        };
        let windows = [
            window(198_000, 58.0, 12_100.0),
            window(341_000, 83.0, 19_800.0),
            window(455_000, 91.0, 24_300.0),
        ];
        let drift = hpcd_measure::drift(&windows);
        let details = [
            "run-queue latency".to_string(),
            "eBPF".into(),
            "3 × 20s".into(),
        ];
        let text = plain(&unstable(
            "order-gateway.service",
            Some("(pid 4821)"),
            &details,
            &windows,
            &drift,
            None,
        ));
        let expected =
            "hpcd measure · order-gateway.service (pid 4821) · run-queue latency · eBPF · 3 × 20s

   window 1/3   p99   198µs   cpu  58%   wakeups 12.1k/s   switches 13.0k/s
   window 2/3   p99   341µs   cpu  83%   wakeups 19.8k/s   switches 13.0k/s
   window 3/3   p99   455µs   cpu  91%   wakeups 24.3k/s   switches 13.0k/s

✘ load is not stable   wakeups +101% and cpu +33 pts across windows
  A percentile now would mix different workloads, not describe one.
  No verdict was given.

  try:
    · wait for steady load, then measure    hpcctl measure … --wait-stable 10m";
        assert_eq!(text, expected);
        assert_eq!(elapsed(Duration::from_secs(130)), "2m 10s");
        assert_eq!(elapsed(Duration::from_secs(600)), "10m");
    }

    #[test]
    fn audit_matches_design() {
        let findings = [
            finding(
                "isolcpus-missing",
                Severity::Info,
                "No isolated cores (requires reboot, proposal only)",
                false,
            ),
            finding(
                "irq-on-hot-core",
                Severity::High,
                "IRQs for eth0 (rx-0..3) are handled on core 3,\nwhere hot thread 4823 runs",
                true,
            ),
            finding(
                "irqbalance-active",
                Severity::Warn,
                "irqbalance is running and would undo IRQ affinity changes",
                false,
            ),
            finding(
                "deep-cstates",
                Severity::High,
                "C6 enabled on cores 2-5 (exit latency 133µs)",
                true,
            ),
        ];
        let details = ["8 threads".to_string()];
        let text = audit(
            "hpcd audit",
            "order-gateway.service",
            Some("(pid 4821)"),
            &details,
            &findings,
            Footer::Fix("hpcctl fix --unit order-gateway.service"),
        );
        let expected = "\
hpcd audit · order-gateway.service (pid 4821) · 8 threads · 4 rules matched

  HIGH   irq-on-hot-core       IRQs for eth0 (rx-0..3) are handled on core 3,
                               where hot thread 4823 runs
  HIGH   deep-cstates          C6 enabled on cores 2-5 (exit latency 133µs)
  WARN   irqbalance-active     irqbalance is running and would undo IRQ affinity changes
  INFO   isolcpus-missing      No isolated cores (requires reboot, proposal only)

2 fixable at runtime · hpcctl explain <rule> for details · hpcctl fix --unit order-gateway.service";
        assert_eq!(plain(&text), expected);
    }

    #[test]
    fn audit_without_findings() {
        let text = audit(
            "hpcd audit",
            "pid 1148",
            None,
            &["1 thread".to_string()],
            &[],
            Footer::Fix("hpcctl fix --pid 1148"),
        );
        assert_eq!(
            plain(&text),
            "hpcd audit · pid 1148 · 1 thread · 0 rules matched\n\n✔ no issues found"
        );
    }

    #[test]
    fn capture_summary() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules/tests/order-gateway");
        let target = hpcd_core::Target::Unit("order-gateway.service".into());
        let captured =
            hpcd_probe::capture(hpcd_probe::Source::snapshot(fixture).unwrap(), &target).unwrap();

        let written = plain(&capture(
            "order-gateway.service",
            Some("(pid 4821)"),
            &captured,
            Some("hpcctl audit --unit order-gateway.service --snapshot gw"),
        ));
        assert!(written.starts_with(
            "hpcd capture · order-gateway.service (pid 4821) · 2 samples 10s apart\n"
        ));
        assert!(written.contains("   from /dev, /proc, /sys · no memory or file contents\n"));
        assert!(
            written
                .ends_with("✔ written · hpcctl audit --unit order-gateway.service --snapshot gw")
        );
        assert!(!written.contains("/proc/interrupts"));

        let shown = plain(&capture("order-gateway.service", None, &captured, None));
        assert!(shown.contains("   t1  /proc/interrupts  "));
        assert!(shown.ends_with("nothing written (--show)"));
    }

    #[test]
    fn firmware_proposal_uses_vendor_steps_or_generic() {
        let rules = hpcd_rules::load_embedded().unwrap();
        let smt: Vec<&Rule> = rules.iter().filter(|r| r.id == "smt-enabled").collect();

        let dell = Machine {
            vendor: Some("Dell Inc.".into()),
            ..Default::default()
        };
        let text = plain(&firmware_proposal(&dell, &smt));
        assert!(text.starts_with("proposal · Dell · System Setup (F2) or iDRAC → BIOS Settings\n"));
        assert!(text.contains("1.  Processor Settings → Logical Processor = Disabled\n"));

        let vm = Machine {
            vendor: Some("Microsoft Corporation".into()),
            ..Default::default()
        };
        let text = plain(&firmware_proposal(&vm, &smt));
        assert!(text.starts_with("proposal · generic · the firmware setup utility\n"));
        assert!(text.ends_with("re-run this audit to verify."));
    }

    #[test]
    fn describes_machine_like_the_design() {
        let m = Machine {
            vendor: Some("Dell Inc.".into()),
            product: Some("PowerEdge R650".into()),
            bios: Some("1.9.2".into()),
            cpu_model: Some("Xeon Gold 6338".into()),
        };
        assert_eq!(
            machine(&m, 2).unwrap(),
            "Dell PowerEdge R650 · BIOS 1.9.2 · 2 × Xeon Gold 6338"
        );
        assert!(machine(&Machine::default(), 0).is_none());
    }

    #[test]
    fn explain_matches_design() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules/irq/irq-on-hot-core.toml");
        let rule = hpcd_rules::load_file(&path).unwrap();
        let expected = "\
irq-on-hot-core · Interrupts handled on a latency-critical core
category irq  ·  severity  HIGH   ·  risk low  ·  reboot no

WHY
   Hardware interrupts preempt whatever is running on the core that handles them.
   If that core runs a latency-critical thread, every interrupt adds jitter.

EVIDENCE
   sources: /proc/interrupts, /proc/irq/*/smp_affinity_list

FIX
   set_irq_affinity → housekeeping cores
   revert  restore each IRQ's previous smp_affinity_list

CONFLICTS
   ⚠ irqbalance: Ban the affected IRQs or the hot cores in irqbalance, or it will undo this fix

REFERENCES
   https://docs.kernel.org/core-api/irq/irq-affinity.html";
        assert_eq!(plain(&explain(&rule)), expected);
    }
}
