mod render;
mod style;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use hpcd_core::Target;
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
    command: Command,
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
    match cli.command {
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

    let qualifier = pid_qualifier(&target, &obs.pids);
    let details = [render::plural(obs.threads.len(), "thread", "threads")];

    let title = if firmware {
        "hpcd firmware audit"
    } else {
        "hpcd audit"
    };
    let fix = format!("hpcctl fix {}", selector(&target));
    anstream::println!(
        "{}",
        render::audit(
            title,
            &target.to_string(),
            qualifier.as_deref(),
            &details,
            &findings,
            &fix
        )
    );
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
