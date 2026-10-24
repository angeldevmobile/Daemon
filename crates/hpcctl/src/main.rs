mod banner;
mod client;
mod inspect;
mod mcp;
mod render;
mod style;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use hpcd_core::ipc::{Request, Response};
use hpcd_core::{Target, parse_duration};
use hpcd_measure::{Assertion, Plan};
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
    /// Plan fixes for a workload, or apply one through the hpcd daemon under a lease.
    Fix {
        #[command(flatten)]
        target: TargetArgs,
        /// Show what would change, without changing anything.
        #[arg(long)]
        dry_run: bool,
        /// Apply this rule's fix through the daemon, e.g. thread-migration.
        #[arg(long, conflicts_with_all = ["dry_run", "snapshot"])]
        rule: Option<String>,
        /// How long the change lasts unless committed, e.g. "10m" (default from policy.toml).
        #[arg(long, value_name = "DURATION", value_parser = parse_duration, requires = "rule")]
        lease: Option<Duration>,
        /// Plan against a capture directory instead of the live system.
        #[arg(long)]
        snapshot: Option<PathBuf>,
        /// Seconds to sample the live system for rates and CPU usage.
        #[arg(long, default_value_t = 3)]
        sample: u64,
    },
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
    ///
    /// Exits 2 without a verdict if the load changes between windows.
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
        /// Keep measuring until the load is steady, for at most this long, e.g. "10m".
        #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
        wait_stable: Option<Duration>,
    },
    /// Serve read-only tools to AI agents over MCP (stdio). Refuses to run as root.
    Mcp {
        /// Read a capture directory instead of the live system.
        #[arg(long)]
        snapshot: Option<PathBuf>,
        /// Seconds to sample the live system for rates and CPU usage.
        #[arg(long, default_value_t = 3)]
        sample: u64,
    },
    /// Show the open leases held by the hpcd daemon.
    Status,
    /// Keep the change of a lease instead of letting it expire.
    Commit { lease: u64 },
    /// Undo the change of a lease now.
    Revert { lease: u64 },
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
        } => {
            let source = source(snapshot, sample)?;
            let text = audit_report(
                cli.rules.as_deref(),
                &target.into_target(),
                source,
                firmware,
            )?;
            anstream::println!("{text}");
        }
        Command::Fix {
            target,
            dry_run,
            rule,
            lease,
            snapshot,
            sample,
        } => {
            let target = target.into_target();
            if let Some(rule) = rule {
                let request = Request::Apply {
                    target,
                    rule,
                    lease_secs: lease.map(|d| d.as_secs()),
                };
                if let Response::Applied { lease } = client::call(&request)? {
                    anstream::println!("{}", render::applied(&lease));
                }
            } else if dry_run {
                let source = source(snapshot, sample)?;
                let text = fix_report(cli.rules.as_deref(), &target, source)?;
                anstream::println!("{text}");
            } else {
                bail!(
                    "use --dry-run to see the plan, then --rule <id> to apply one fix under a lease"
                );
            }
        }
        Command::Status => {
            if let Response::Status { leases } = client::call(&Request::Status)? {
                anstream::println!("{}", render::leases(&leases));
            }
        }
        Command::Commit { lease } => {
            client::call(&Request::Commit { lease })?;
            anstream::println!(
                "{}",
                style::paint(
                    style::OK_BOLD,
                    format!("✔ lease {lease} committed · the change stays until reboot")
                )
            );
        }
        Command::Revert { lease } => {
            client::call(&Request::Revert { lease })?;
            anstream::println!(
                "{}",
                style::paint(style::OK_BOLD, format!("✔ lease {lease} reverted"))
            );
        }
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
            wait_stable,
        } => {
            let tty = std::io::stderr().is_terminal();
            let (text, code) = measure_report(
                target.into_target(),
                windows,
                window,
                wait_stable,
                &asserts,
                tty,
            )?;
            anstream::println!("{text}");
            if code != 0 {
                std::process::exit(code);
            }
        }
        Command::Mcp { snapshot, sample } => mcp::serve(mcp::Server {
            rules: cli.rules,
            snapshot,
            sample: Duration::from_secs(sample),
        })?,
        Command::Checks => print_entries(registry::CHECKS.iter().map(|c| (c.name, c.description))),
        Command::Actions => {
            print_entries(registry::ACTIONS.iter().map(|a| (a.name, a.description)))
        }
    }
    Ok(())
}

fn source(snapshot: Option<PathBuf>, sample: u64) -> Result<Source> {
    match snapshot {
        Some(dir) => {
            Source::snapshot(&dir).with_context(|| format!("opening capture {}", dir.display()))
        }
        None if !cfg!(target_os = "linux") => {
            bail!("the live system needs Linux; use --snapshot <dir> to read a capture")
        }
        None => Ok(Source::live(Duration::from_secs(sample))),
    }
}

fn fix_report(rules_dir: Option<&Path>, target: &Target, source: Source) -> Result<String> {
    let rules: Vec<Rule> = load_rules(rules_dir)?
        .into_iter()
        .filter(|r| r.category != Category::Firmware)
        .collect();
    let obs = hpcd_probe::observe(&source, target)?;
    let findings = hpcd_rules::evaluate(&rules, &obs)?;
    let planned = hpcd_rules::plan(&rules, &findings, &obs);
    Ok(render::fix_plan(
        &target.to_string(),
        pid_qualifier(target, &obs.pids).as_deref(),
        &planned,
    ))
}

fn audit_report(
    rules_dir: Option<&Path>,
    target: &Target,
    source: Source,
    firmware: bool,
) -> Result<String> {
    let rules: Vec<Rule> = load_rules(rules_dir)?
        .into_iter()
        .filter(|r| (r.category == Category::Firmware) == firmware)
        .collect();
    let obs = hpcd_probe::observe(&source, target)?;
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
        let fix = format!("hpcctl fix {}", selector(target));
        render::audit(
            "hpcd audit",
            &target.to_string(),
            pid_qualifier(target, &obs.pids).as_deref(),
            &[render::plural(obs.threads.len(), "thread", "threads")],
            &findings,
            render::Footer::Fix(&fix),
        )
    };
    let mut text = text;
    if firmware && obs.smis.is_none() {
        text += &format!(
            "
{}",
            style::paint(
                style::DIM,
                "smi-activity not checked: needs root and the msr module (Intel only)"
            )
        );
    }
    Ok(text)
}

fn measure_report(
    target: Target,
    windows: u32,
    window: u64,
    wait_stable: Option<Duration>,
    asserts: &[String],
    tty: bool,
) -> Result<(String, i32)> {
    let asserts = asserts
        .iter()
        .map(|a| a.parse::<Assertion>().map_err(anyhow::Error::msg))
        .collect::<Result<Vec<_>>>()?;
    let plan = Plan {
        windows: windows.max(1) as usize,
        window: Duration::from_secs(window),
        wait_stable,
    };
    let total = plan.needed();
    let m = hpcd_measure::measure(&target, plan, |done| {
        if tty {
            let text = if done > total {
                format!(
                    "waiting for steady load {}",
                    render::elapsed(Duration::from_secs(done as u64))
                )
            } else {
                format!("measuring {done}/{total}s")
            };
            anstream::eprint!("\r\x1b[2K{}", style::paint(style::DIM, text));
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

    let qualifier = pid_qualifier(&target, &m.pids);
    let details = [
        "run-queue latency".to_string(),
        "eBPF".to_string(),
        format!("{windows} × {window}s"),
    ];
    let measured = m.windows();
    let drift = hpcd_measure::drift(&measured);
    if !drift.is_empty() {
        let text = render::unstable(
            &target.to_string(),
            qualifier.as_deref(),
            &details,
            &measured,
            &drift,
            wait_stable,
        );
        return Ok((text, 2));
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

    let mut summary = format!(
        "{} · {}",
        render::plural(m.wakeups() as usize, "wakeup", "wakeups"),
        render::plural(m.threads, "thread", "threads"),
    );
    if measured.len() > 1 {
        summary += " · load stable";
        if !m.waited.is_zero() {
            summary += &format!(" after {}", render::elapsed(m.waited));
        }
        summary += " ✔";
    }
    let text = render::measure(
        &target.to_string(),
        qualifier.as_deref(),
        &details,
        &summary,
        Some(&m.guardrails()),
        Some(&m.overhead),
        &rows,
    );
    let failed = rows.iter().any(|r| r.check.is_some_and(|(_, ok)| !ok));
    Ok((text, i32::from(failed)))
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
