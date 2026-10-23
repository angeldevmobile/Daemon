use hpcd_core::Finding;
use hpcd_probe::Capture;
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

    section(
        &mut out,
        "REFERENCES",
        rule.references.links.iter().map(|l| paint(style::LINK, l)),
    );
    out.join("\n")
}

pub fn audit(
    title: &str,
    subject: &str,
    qualifier: Option<&str>,
    details: &[String],
    findings: &[Finding],
    fix_command: &str,
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
    out.push(String::new());
    out.push(footer);
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
    format!("{n} {}", if n == 1 { one } else { many })
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
            "hpcctl fix --unit order-gateway.service",
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
            "hpcctl fix --pid 1148",
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
        assert!(written.contains("   from /proc, /sys · no memory or file contents\n"));
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
