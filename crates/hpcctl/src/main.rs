mod banner;
mod render;
mod style;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use hpcd_core::Target;
use hpcd_measure::Assertion;
use hpcd_probe::Source;
use hpcd_rules::{Category, Rule, registry};

#[derive(Parser)]
#[command(
    name = "hpcctl",
    version,
    about = "Find out why your workload is jittery. Fix it. Prove it."
)]
struct Cli {
    /// Load rules from this directory instead of the built-in set.
    #[arg(long, global = true, env = "HPCD_RULES")]
    rules: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Diagnose a workload (read-only).
    Audit {
        #[command(flatten)]
        target: TargetArgs,
        /// Audit a capture directory instead of the live system.
        #[arg(long)]
        snapshot: Option<PathBuf>,
        /// Audit firmware and BIOS settings instead (detect and propose only).
        #[arg(long)]
        firmware: bool,
        /// Seconds to sample the live system for rates and CPU usage.
        #[arg(long, default_value_t = 3)]
        sample: u64,
    },
    /// Show the full explanation and references for a rule.
    Explain { rule: String },
    /// Capture /proc and /sys data for offline audit.
    Capture {
        #[command(flatten)]
        target: TargetArgs,
        /// Directory to write the capture to (must not exist or be empty).
        #[arg(short, long)]
        output: PathBuf,
        /// Seconds between the two samples.
        #[arg(long, default_value_t = 3)]
        sample: u64,
        /// List every file the capture would contain, without writing anything.
        #[arg(long)]
        show: bool,
    },
    /// Measure run-queue latency with eBPF (needs root, or CAP_BPF and CAP_PERFMON).
    Measure {
        #[command(flatten)]
        target: TargetArgs,
        /// Number of measurement windows.
        #[arg(long, default_value_t = 5)]
        windows: u32,
        /// Seconds per window.
        #[arg(long, default_value_t = 20)]
        window: u64,
        /// Exit 1 unless the condition holds, e.g. "p99 < 200µs". Repeatable.
        #[arg(long = "assert", value_name = "CONDITION")]
        asserts: Vec<String>,
    },
    /// List the checks rules can use.
    Checks,
    /// List the actions rules can use.
    Actions,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct TargetArgs {
    #[arg(long)]
    unit: Option<String>,
    #[arg(long)]
    cgroup: Option<PathBuf>,
    #[arg(long)]
    exe: Option<PathBuf>,
    #[arg(long)]
    pid: Option<u32>,
}

impl TargetArgs {
    fn into_target(self) -> Target {
        match (self.unit, self.cgroup, self.exe, self.pid) {
            (Some(u), ..) => Target::Unit(u),
            (_, Some(c), ..) => Target::Cgroup(c),
            (_, _, Some(e), _) => Target::Exe(e),
            (.., Some(p)) => Target::Pid(p),
            _ => unreachable!("clap requires exactly one target selector"),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        if std::io::stdout().is_terminal() {
            anstream::println!("{}", banner::render());
        } else {
            Cli::command().print_help()?;
        }
        return Ok(());
    };
    match command {
        Command::Audit {
            target,
            snapshot,
            firmware,
            sample,
        } => audit(
            cli.rules.as_deref(),
            target.into_target(),
            snapshot,
            firmware,
            sample,
        )?,
        Command::Explain { rule } => {
            let rules = load_rules(cli.rules.as_deref())?;
            let rule = rules
                .iter()
                .find(|r| r.id == rule)
                .with_context(|| format!("no rule named '{rule}'"))?;
            anstream::println!("{}", render::explain(rule));
        }
        Command::Capture {
            target,
            output,
            sample,
            show,
        } => capture(target.into_target(), &output, sample, show)?,
        Command::Measure {
            target,
            windows,
            window,
            asserts,
        } => measure(target.into_target(), windows, window, &asserts)?,
        Command::Checks => print_entries(registry::CHECKS.iter().map(|c| (c.name, c.description))),
        Command::Actions => {
            print_entries(registry::ACTIONS.iter().map(|a| (a.name, a.description)))
        }
    }
    Ok(())
}

fn audit(
    rules_dir: Option<&Path>,
    target: Target,
    snapshot: Option<PathBuf>,
    firmware: bool,
    sample: u64,
) -> Result<()> {
    let rules: Vec<Rule> = load_rules(rules_dir)?
        .into_iter()
        .filter(|r| (r.category == Category::Firmware) == firmware)
        .collect();

    let source = match snapshot {
        Some(dir) => {
            Source::snapshot(&dir).with_context(|| format!("opening capture {}", dir.display()))?
        }
        None if !cfg!(target_os = "linux") => {
            bail!("live audit needs Linux; use --snapshot <dir> to audit a capture")
        }
        None => Source::live(Duration::from_secs(sample)),
    };
    let obs = hpcd_probe::observe(&source, &target)?;
    let findings = hpcd_rules::evaluate(&rules, &obs)?;

    let text = if firmware {
        let matched: Vec<&Rule> = rules
            .iter()
            .filter(|r| findings.iter().any(|f| f.rule_id == r.id))
            .collect();
        let subject =
            render::machine(&obs.machine, obs.packages()).unwrap_or_else(|| target.to_string());
        render::audit(
            "hpcd firmware audit",
            &subject,
            None,
            &[],
            &findings,
            render::Footer::Text(render::firmware_proposal(&obs.machine, &matched)),
        )
    } else {
        let fix = format!("hpcctl fix {}", selector(&target));
        render::audit(
            "hpcd audit",
            &target.to_string(),
            pid_qualifier(&target, &obs.pids).as_deref(),
            &[render::plural(obs.threads.len(), "thread", "threads")],
            &findings,
            render::Footer::Fix(&fix),
        )
    };
    anstream::println!("{text}");
    Ok(())
}

fn measure(target: Target, windows: u32, window: u64, asserts: &[String]) -> Result<()> {
    let asserts = asserts
        .iter()
        .map(|a| a.parse::<Assertion>().map_err(anyhow::Error::msg))
        .collect::<Result<Vec<_>>>()?;
    let duration = Duration::from_secs(window * u64::from(windows.max(1)));
    let tty = std::io::stderr().is_terminal();
    let m = hpcd_measure::measure(&target, duration, |done, total| {
        if tty {
            anstream::eprint!(
                "\r{}",
                style::paint(style::DIM, format!("measuring {done}/{total}s"))
            );
        }
    })?;
    if tty {
        anstream::eprint!("\r\x1b[2K");
    }
    if m.wakeups() == 0 {
        let c = m.counters;
        bail!(
            "no wakeups recorded for {target} ({} threads watched; tracepoints saw {} wakeups and {} switches, {} runnable events for the target, {} recorded)",
            m.threads,
            c.wakeups,
            c.switches,
            c.target_runnable,
            c.recorded
        );
    }

    let mut quantiles: Vec<(String, f64)> = [("p50", 0.5), ("p99", 0.99), ("p99.9", 0.999)]
        .map(|(l, q)| (l.to_string(), q))
        .into();
    for a in &asserts {
        if !quantiles.iter().any(|(l, _)| *l == a.label) {
            quantiles.push((a.label.clone(), a.quantile));
        }
    }
    let rows: Vec<render::LatencyRow> = quantiles
        .into_iter()
        .filter(|(l, _)| asserts.is_empty() || asserts.iter().any(|a| a.label == *l))
        .filter_map(|(label, q)| {
            let estimate = hpcd_measure::estimate(&m.blocks, q)?;
            let check = asserts
                .iter()
                .find(|a| a.label == label)
                .map(|a| (a.limit_ns, a.holds(estimate.value)));
            Some(render::LatencyRow {
                label,
                estimate,
                check,
            })
        })
        .collect();

    let qualifier = pid_qualifier(&target, &m.pids);
    let details = [
        "run-queue latency".to_string(),
        "eBPF".to_string(),
        format!("{windows} × {window}s"),
    ];
    let summary = format!(
        "{} · {}",
        render::plural(m.wakeups() as usize, "wakeup", "wakeups"),
        render::plural(m.threads, "thread", "threads"),
    );
    anstream::println!(
        "{}",
        render::measure(
            &target.to_string(),
            qualifier.as_deref(),
            &details,
            &summary,
            &rows
        )
    );
    if rows.iter().any(|r| r.check.is_some_and(|(_, ok)| !ok)) {
        std::process::exit(1);
    }
    Ok(())
}

fn capture(target: Target, output: &Path, sample: u64, show: bool) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("capture needs Linux");
    }
    let captured = hpcd_probe::capture(Source::live(Duration::from_secs(sample)), &target)?;
    let qualifier = pid_qualifier(&target, &captured.observation.pids);
    let audit = format!(
        "hpcctl audit {} --snapshot {}",
        selector(&target),
        output.display()
    );
    if !show {
        captured.write(output)?;
    }
    anstream::println!(
        "{}",
        render::capture(
            &target.to_string(),
            qualifier.as_deref(),
            &captured,
            (!show).then_some(audit.as_str()),
        )
    );
    Ok(())
}

fn pid_qualifier(target: &Target, pids: &[u32]) -> Option<String> {
    if matches!(target, Target::Pid(_)) {
        return None;
    }
    let list: Vec<String> = pids.iter().map(u32::to_string).collect();
    let label = if list.len() == 1 { "pid" } else { "pids" };
    Some(format!("({label} {})", list.join(", ")))
}

fn selector(target: &Target) -> String {
    match target {
        Target::Unit(u) => format!("--unit {u}"),
        Target::Cgroup(p) => format!("--cgroup {}", p.display()),
        Target::Exe(p) => format!("--exe {}", p.display()),
        Target::Pid(p) => format!("--pid {p}"),
    }
}

fn load_rules(dir: Option<&Path>) -> Result<Vec<Rule>> {
    match dir {
        Some(dir) => hpcd_rules::load_dir(dir)
            .with_context(|| format!("loading rules from {}", dir.display())),
        None => hpcd_rules::load_embedded().context("loading built-in rules"),
    }
}

fn print_entries(entries: impl Iterator<Item = (&'static str, &'static str)> + Clone) {
    let width = entries.clone().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, description) in entries {
        anstream::println!(
            "{}  {description}",
            style::paint(style::ID, format!("{name:width$}"))
        );
    }
}
