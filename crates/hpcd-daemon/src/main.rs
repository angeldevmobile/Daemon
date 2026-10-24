#[cfg(target_os = "linux")]
mod access;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod leases;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod policy;
#[cfg(target_os = "linux")]
mod server;

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "hpcd",
    version,
    about = "The hpcd daemon: applies fixes under leases and reverts them"
)]
pub struct Args {
    /// Policy file.
    #[arg(long, default_value = "/etc/hpcd/policy.toml")]
    config: PathBuf,
    /// Load rules from this directory instead of the built-in set.
    #[arg(long, env = "HPCD_RULES")]
    rules: Option<PathBuf>,
    /// Unix socket to listen on.
    #[arg(long, env = "HPCD_SOCKET", default_value = hpcd_core::ipc::DEFAULT_SOCKET)]
    socket: PathBuf,
    /// Directory for the change journal.
    #[arg(long, default_value = "/var/lib/hpcd")]
    state: PathBuf,
    /// Seconds to sample the system before planning a fix.
    #[arg(long, default_value_t = 3)]
    sample: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    #[cfg(target_os = "linux")]
    {
        server::run(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("hpcd needs Linux")
    }
}
