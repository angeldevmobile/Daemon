use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use hpcd_core::Target;
use hpcd_core::ipc::{LeaseInfo, Request, Response};
use hpcd_fix::{Fixer, Live};
use hpcd_probe::Source;
use hpcd_rules::Rule;

use crate::Args;
use crate::access;
use crate::leases::Leases;
use crate::policy::Policy;

struct Shared {
    leases: Mutex<Leases<Live>>,
    wake: Condvar,
    policy: Policy,
    rules: Vec<Rule>,
    sample: Duration,
    groups: Vec<u32>,
    euid: u32,
}

impl Shared {
    fn leases(&self) -> MutexGuard<'_, Leases<Live>> {
        self.leases.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn log(message: impl std::fmt::Display) {
    eprintln!("hpcd: {message}");
}

pub fn run(args: Args) -> Result<()> {
    let text = std::fs::read_to_string(&args.config)
        .with_context(|| format!("reading {}", args.config.display()))?;
    let policy = Policy::parse(&text)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("policy {}", args.config.display()))?;
    let rules = match &args.rules {
        Some(dir) => hpcd_rules::load_dir(dir)?,
        None => hpcd_rules::load_embedded()?,
    };
    let (fixer, recovery) = Fixer::new(Live::new(), args.state.join("journal"))?;
    for change in &recovery.reverted {
        log(format!("reverted after an unclean stop: {change}"));
    }
    for (change, error) in &recovery.failed {
        log(format!(
            "could not revert after an unclean stop: {change}: {error}"
        ));
    }
    let groups = policy
        .access
        .groups
        .iter()
        .filter_map(|name| {
            let gid = access::group_id(name);
            if gid.is_none() {
                log(format!("group '{name}' from [access] does not exist"));
            }
            gid
        })
        .collect();

    let signals = block_signals();
    let listener = bind(&args.socket)?;
    let shared = Arc::new(Shared {
        leases: Mutex::new(Leases::new(fixer)),
        wake: Condvar::new(),
        policy,
        rules,
        sample: Duration::from_secs(args.sample),
        groups,
        euid: unsafe { libc::geteuid() },
    });
    {
        let shared = shared.clone();
        std::thread::spawn(move || expire_leases(&shared));
    }
    {
        let shared = shared.clone();
        let socket = args.socket.clone();
        std::thread::spawn(move || stop_on_signal(&shared, signals, &socket));
    }
    notify_ready();
    log(format!("listening on {}", args.socket.display()));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let shared = shared.clone();
                std::thread::spawn(move || handle(&shared, stream));
            }
            Err(e) => log(format!("accept: {e}")),
        }
    }
    Ok(())
}

fn bind(path: &Path) -> Result<UnixListener> {
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            bail!("another hpcd is already listening on {}", path.display());
        }
        std::fs::remove_file(path).with_context(|| format!("removing stale {}", path.display()))?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    Ok(listener)
}

fn block_signals() -> libc::sigset_t {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, sig);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

fn stop_on_signal(shared: &Shared, set: libc::sigset_t, socket: &PathBuf) {
    let mut sig = 0;
    unsafe { libc::sigwait(&set, &mut sig) };
    let mut leases = shared.leases();
    log(format!("signal {sig}: reverting open leases"));
    for (id, result) in leases.revert_all() {
        match result {
            Ok(()) => log(format!("lease {id} reverted")),
            Err(e) => log(format!("lease {id} could not be reverted: {e}")),
        }
    }
    let _ = std::fs::remove_file(socket);
    std::process::exit(0);
}

fn expire_leases(shared: &Shared) {
    let mut leases = shared.leases();
    loop {
        leases = match leases.next_expiry() {
            None => shared.wake.wait(leases).unwrap_or_else(|e| e.into_inner()),
            Some(due) => {
                let now = Instant::now();
                if due > now {
                    shared
                        .wake
                        .wait_timeout(leases, due - now)
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                } else {
                    for (id, result) in leases.expire(now) {
                        match result {
                            Ok(()) => log(format!("lease {id} expired and was reverted")),
                            Err(e) => log(format!("lease {id} expired, revert failed: {e}")),
                        }
                    }
                    leases
                }
            }
        };
    }
}

fn notify_ready() {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    let Ok(socket) = UnixDatagram::unbound() else {
        return;
    };
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str());
    let sent = match bytes.strip_prefix(b"@") {
        Some(name) => {
            use std::os::linux::net::SocketAddrExt;
            std::os::unix::net::SocketAddr::from_abstract_name(name)
                .and_then(|addr| socket.send_to_addr(b"READY=1", &addr))
        }
        None => socket.send_to(b"READY=1", &path),
    };
    if let Err(e) = sent {
        log(format!("notifying systemd: {e}"));
    }
}

fn handle(shared: &Shared, stream: UnixStream) {
    let uid = match access::peer_uid(&stream) {
        Ok(uid) => uid,
        Err(e) => return log(format!("SO_PEERCRED: {e}")),
    };
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let response = match serde_json::from_str::<Request>(&line) {
        Ok(request) => dispatch(shared, uid, request),
        Err(e) => Response::Error {
            message: format!("bad request: {e}"),
        },
    };
    let mut out = &stream;
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = writeln!(out, "{json}");
    }
}

fn authorize(shared: &Shared, uid: u32) -> Result<(), String> {
    if uid == 0 || uid == shared.euid {
        return Ok(());
    }
    if access::groups_of(uid)
        .iter()
        .any(|g| shared.groups.contains(g))
    {
        return Ok(());
    }
    Err(format!(
        "uid {uid} may not change the system: add the user to a group listed in [access] of policy.toml"
    ))
}

fn dispatch(shared: &Shared, uid: u32, request: Request) -> Response {
    let result = match request {
        Request::Status => {
            return Response::Status {
                leases: shared.leases().status(Instant::now()),
            };
        }
        Request::Apply {
            target,
            rule,
            lease_secs,
        } => apply(shared, uid, target, &rule, lease_secs).map(|lease| Response::Applied { lease }),
        Request::Commit { lease } => authorize(shared, uid)
            .and_then(|()| shared.leases().commit(lease, uid))
            .map(|()| {
                log(format!("lease {lease} committed by uid {uid}"));
                Response::Committed { lease }
            }),
        Request::Revert { lease } => {
            let result = shared.leases().revert(lease, uid);
            shared.wake.notify_all();
            result.map(|()| {
                log(format!("lease {lease} reverted by uid {uid}"));
                Response::Reverted { lease }
            })
        }
    };
    result.unwrap_or_else(|message| Response::Error { message })
}

fn apply(
    shared: &Shared,
    uid: u32,
    target: Target,
    rule: &str,
    lease_secs: Option<u64>,
) -> Result<LeaseInfo, String> {
    authorize(shared, uid)?;
    let duration = shared.policy.lease(lease_secs)?;
    if !shared.rules.iter().any(|r| r.id == rule) {
        return Err(format!("no rule named '{rule}'"));
    }
    let obs =
        hpcd_probe::observe(&Source::live(shared.sample), &target).map_err(|e| e.to_string())?;
    let findings = hpcd_rules::evaluate(&shared.rules, &obs).map_err(|e| e.to_string())?;
    let plan = hpcd_rules::plan(&shared.rules, &findings, &obs)
        .into_iter()
        .find(|p| p.rule_id == rule)
        .ok_or_else(|| format!("{rule} does not match {target} right now"))?
        .plan?;
    let lease = shared
        .leases()
        .apply(target, plan, uid, duration, Instant::now())?;
    shared.wake.notify_all();
    log(format!(
        "lease {} opened by uid {uid}: {} → {} · {} changes · expires in {}s",
        lease.id,
        lease.plan.rule_id,
        lease.plan.summary,
        lease.plan.changes.len(),
        lease.expires_in_secs
    ));
    Ok(lease)
}
