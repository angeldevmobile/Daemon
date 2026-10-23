<div align="center">

# hpcd — High-Performance Compute Daemon

**Find out why your workload is jittery. Fix it. Prove it.**

An open source Rust daemon that diagnoses tail latency on Linux, applies verified fixes,
and rolls them back if they don't help. Driven by open rules anyone can read and extend.
Usable from the CLI or by AI agents over MCP.

</div>

> **Status: pre-alpha.** hpcd is under active design and development. Commands, the rule
> format and the IPC protocol will change. Do not run it on production machines yet.

<!-- TODO: demo GIF — jitter-demo p99 before/after, agent diagnosing and applying fixes -->

---

## Why

Your mean latency is fine. Your p99 is not. The cause is usually not your code: it's an IRQ
landing on the core your hot thread runs on, a CPU waking up from a deep C-state, a governor
scaling frequency down, or memory living on the wrong NUMA node.

The knowledge to fix this exists, but it is scattered across outdated guides, vendor PDFs and
the heads of a few engineers. Instructions change with every distro, CPU and kernel version.
Changes get applied by hand, never measured, never documented, and lost on the next reboot.

hpcd turns that into a closed loop:

```
diagnose → explain → plan → apply → measure → commit or roll back
```

hpcd is built for **latency-sensitive workloads** on a single machine: trading systems,
telecom and packet processing, real-time audio, game servers, low-latency APIs. It is not a
cluster or batch-scheduling tool.

The same engine has a second use that every developer runs into: **noisy benchmarks**.
`hpcctl run -- cargo bench` quiets the machine for the duration of one command and restores
it afterwards. See [Benchmark mode](#benchmark-mode).

## Principles

**1. Open rules.** Every check and every fix is a readable, versioned rule in this repository.
No hidden tuning. Anyone can audit what hpcd does, and anyone can contribute what they know.

**2. Explain everything.** Every finding comes with the evidence that triggered it and a
reference explaining why it matters.

**3. Verify every change.** Fixes are measured against a baseline with repeated trials and a
confidence interval. If a change does not clearly help, it is reverted.

**4. Safe by construction.** Read-only by default. Every change has a snapshot and a lease.
If it is not confirmed in time, it is undone automatically.

**5. Start from the workload, not the machine.** Point hpcd at the service that hurts and it
finds what is affecting it.

## How it compares

| Tool | What it does | What hpcd adds |
|---|---|---|
| [`tuned`](https://github.com/redhat-performance/tuned) | Applies system-wide profiles (e.g. `latency-performance`) | Per-workload diagnosis, evidence, measurement, automatic rollback |
| [`tuna`](https://git.kernel.org/pub/scm/utils/tuna/tuna.git) | Manual control of IRQ and thread affinity | Decides *what* to move and verifies it helped |
| [`rtla`](https://docs.kernel.org/tools/rtla/index.html) / `osnoise` | Measures OS noise and timer latency | Links noise to a cause and a fix for a specific workload |
| `irqbalance` | Spreads IRQs across cores for throughput | Keeps IRQs off latency-critical cores, and detects when irqbalance undoes it |
| `perf sched`, BCC `runqlat` | Raw scheduler measurements | Turns them into findings, fixes and a before/after verdict |
| [`pyperf system tune`](https://pyperf.readthedocs.io/en/latest/system.html) | Tunes the machine for benchmarking | Scoped to one command, reverted on exit even after a crash, variance measured |

hpcd does not replace these tools. It detects when they are active, works alongside them and
reports when they would undo one of its changes.

## Example

```console
$ hpcctl audit --unit order-gateway.service

hpcd audit · order-gateway.service (pid 4821) · 8 threads · 8 rules matched

 HIGH  irq-on-hot-core       IRQs for eth0 (rx-0..3) are handled on core 3,
                             where hot thread 4823 runs
 HIGH  deep-cstates          C6 enabled on cores 2-5 (exit latency 133µs)
 MED   powersave-governor    Governor 'powersave' on cores 2-5
 MED   thread-migration      Thread 4823 migrated across 4 cores in 10s
 MED   smt-sibling-busy      kworker activity on core 7, SMT sibling of core 3
 LOW   remote-numa-memory    41% of resident memory on node 1, thread runs on node 0
 WARN  irqbalance-active     irqbalance is running and would undo IRQ affinity changes
 INFO  isolcpus-missing      No isolated cores (requires reboot, proposal only)

5 fixable at runtime · `hpcctl explain <rule>` for details · `hpcctl fix --unit order-gateway.service`
```

```console
$ hpcctl fix --unit order-gateway.service --interactive

baseline  run-queue latency p99: 212µs  [205–219]  (eBPF, 5 × 20s, load stable ✔)
          throughput 18.4k req/s · cpu 61% · package power 142W · rss 3.1 GiB

[1/5] irq-on-hot-core → move eth0 IRQs to cores 0-1          apply? [y/N] y
      p99 141µs [136–147]  -33%  ✔ kept       side effects: none          lease 5m
[2/5] deep-cstates → hold /dev/cpu_dma_latency at 0          apply? [y/N] y
      p99  48µs [45–51]    -66%  ✔ kept       side effects: power +18W ⚠  lease 5m
[3/5] powersave-governor → 'performance' on cores 2-5        apply? [y/N] y
      p99  47µs [44–51]     -2%  ✘ within noise · reverted
[4/5] thread-migration → pin thread 4823 to core 3           apply? [y/N] y
      p99  41µs [38–44]    -15%  ✘ guardrail: throughput -11% (limit -5%) · reverted
[5/5] smt-sibling-busy → move kworker off core 7             apply? [y/N] y
      p99  45µs [39–51]     -6%  ? inconclusive · keep anyway? [y/N] n · reverted

2 kept · 3 reverted · leases expire in 4m 12s · `hpcctl commit` to persist
```

## Measurement

Measurement is what makes hpcd different from a tuning script, so it gets the most care.

**What is measured.** By default, per-thread **run-queue latency**: the time between a hot
thread becoming runnable and actually running, collected as a histogram with eBPF from the
`sched_wakeup` and `sched_switch` tracepoints. This captures IRQ, C-state, frequency and
migration effects without touching the application. When the application exposes its own
latency (a Prometheus endpoint or a command that prints a percentile), hpcd can use that
instead with `--metric`.

**How a verdict is reached.**

1. **Stability check.** Before measuring, hpcd watches load (CPU usage, context switches,
   optionally request rate). If the workload is not in a steady state, it refuses to give a
   verdict instead of giving a wrong one.
2. **Repeated, interleaved trials.** Each candidate is measured over several short windows,
   alternating with the baseline (A/B/A/B) so that slow drift affects both sides equally.
3. **Confidence intervals.** The p99 of each side is estimated with a bootstrap confidence
   interval. A fix is kept only if the improvement is significant *and* larger than a
   minimum effect size (5% by default).
4. **Guardrails.** Lower latency is not free. Holding cores out of deep C-states costs power,
   pinning threads can cost throughput. Every trial also records throughput (request rate
   with `--metric`, otherwise CPU work done), CPU usage, package power (RAPL), memory (RSS,
   page faults, NUMA misses) and temperature. A fix that improves p99 but breaks a guardrail
   is reverted. Limits are set in `policy.toml`:

   ```toml
   [guardrails]
   throughput_drop = "5%"
   power_increase  = "25W"
   cpu_increase    = "10pts"
   rss_increase    = "10%"
   ```

   Side effects within the limits are kept but reported, so the cost of every fix is visible.
5. **Interactions.** Fixes are not additive: holding C-states can hide the benefit of a
   governor change. Fixes are tried in the order each rule declares, and at the end a
   **leave-one-out** pass removes each kept fix once to check it still contributes.

If hpcd cannot measure reliably, it says so. "Inconclusive" is a valid result.

### Overhead

A tool that measures latency must not add it. hpcd keeps its own cost small and reports it:

- **Idle:** the daemon sleeps on its socket and on netlink/udev events. No polling, no
  tracing. Target: under 0.1% CPU and 20 MiB RSS.
- **Measuring:** eBPF programs are attached only during measurement windows and detached
  afterwards. Scheduler tracepoints cost more on workloads with very high context-switch
  rates, so hpcd measures its own overhead and includes it in every report.
- **Audit:** reads `/proc` and `/sys` once per run. No background collection.

## Benchmark mode

Benchmarks that move ±10% between runs hide the 3% regression you are looking for. Most of
that noise comes from the machine, not the code: frequency scaling, turbo, C-state wake-ups,
interrupts and other work on the same cores.

```console
$ hpcctl run --repeat 5 -- cargo bench -p parser

quiet profile · cores 4-7 for the command · 6 changes · lease bound to the command
  governor 'performance' + fixed frequency on 4-7 · turbo off · C-states held
  IRQs moved to 0-3 · SMT siblings of 4-7 idle · command pinned to 4-7

run 1/5 … 5/5
restored 6 changes

run-to-run variance   9.8% → 1.2%   (coefficient of variation of the median, 5 runs each)
```

- **Scoped to one command.** The lease is tied to the child process through a `pidfd`. When
  the command exits, crashes or is killed, the daemon restores every change.
- **Measured.** With `--repeat`, hpcd runs the command with and without the quiet profile and
  reports the variance of each, so you can see that it helped.
- **Comparable, not representative.** Turbo off and fixed frequency make results stable
  across runs and commits, but slower than production. Use it to compare, not to quote
  absolute numbers.
- **Same rules.** Rules declare which goals they serve (`goal = ["latency", "stability"]`).
  Benchmark mode uses the `stability` rules; there is no separate tuning logic.

Works in CI on dedicated runners: `hpcctl run --check-variance 2% -- <bench>` fails the job
when the machine is too noisy for the results to mean anything.

## Testing latency

Tail latency is usually discovered in production. hpcd makes it something you can test.

**Assert in CI.** Fail a pipeline when a change makes the tail worse:

```console
$ hpcctl measure --unit api.service --assert "p99 < 200µs" --assert "p99.9 < 1ms"
p99 182µs [176–189] ✔ · p99.9 1.31ms [1.18–1.44] ✘ · exit 1
```

**Inject noise.** Reproduce production jitter on purpose, to check that the application
copes, that alerts fire, or that a fix really works:

```console
$ hpcctl noise --irq-storm core:3 --cstates deep --smt-sibling core:3 --duration 60s
```

Noise injection is the mirror of `fix`: same actions, same snapshots and leases, reverted
when the duration ends. It only runs on hosts where `policy.toml` sets `allow_noise = true`,
which the default policy does not.

**Audit offline.** Capture a machine once and audit it anywhere, without access to it:

```console
customer$ hpcctl capture --unit api.service -o api-host
you$      hpcctl audit --unit api.service --snapshot api-host
```

A capture is a directory with two samples of exactly the files the audit reads: `/proc` and
`/sys` data plus tuned's active profile name, never memory, file contents or command lines.
`hpcctl capture --show` lists every file it would include without writing anything.

## Targets

A PID disappears when a process restarts. Fixes are attached to a **stable target** instead:

| Selector | Example |
|---|---|
| `--unit` | `--unit order-gateway.service` |
| `--cgroup` | `--cgroup /sys/fs/cgroup/trading.slice` |
| `--exe` | `--exe /opt/gateway/bin/gateway` |
| `--pid` | `--pid 4821` (one-off; cannot be committed) |

Hot threads within a target are found automatically (by CPU time and wakeup rate) or named
with `--thread <name-pattern>`.

A **committed** fix becomes part of the target's profile. The daemon re-applies it when the
target starts again (process events via the netlink proc connector) and when the system
resets it (for example, IRQ affinity after a NIC driver reload, detected through udev).

## Always-on protection

A one-time fix decays. A kernel update resets the governor, a new monitoring agent lands on
the hot core, a cron job starts at 03:00. The daemon keeps watching after the fix is done.

### Watch

```console
$ hpcctl watch --unit order-gateway.service

watching order-gateway.service · profile: 2 committed fixes · shield: cores 2-5

03:00:12  DRIFT   governor on cores 2-5 changed to 'powersave' (package upgrade: tuned)
          ↳ re-applied 'performance' · profile restored                       auto
03:00:14  NOISE   new process backup-agent (pid 22817) scheduled on core 3
          ↳ moved to housekeeping cores 0-1                                    shield
03:04:51  NOISE   eth0 rx-2 IRQ re-routed to core 4 after link flap
          ↳ moved back to cores 0-1                                            auto
09:12:30  ALERT   p99 312µs over the last 5 min (profile baseline 48µs)
          ↳ no known cause found · notified: journald, webhook, mcp
```

- **Drift:** committed settings that changed under hpcd. Re-applied if policy allows,
  reported otherwise, with what changed them when it can be traced.
- **Noise:** new sources of jitter on protected cores: processes, kworkers, timers, IRQs.
- **Alerts:** optional periodic measurement against the profile's baseline. Off by default.
- **Notifications** go to journald, a webhook, or the agent as an MCP notification. hpcd has
  no dashboard of its own; it reports to the tools you already use.

Watching is event-driven (netlink proc connector, udev, cgroup events) plus a cheap check of a
few `/sys` files every 30 seconds. Idle cost stays within the targets in [Overhead](#overhead).

### Shield

Linux already balances load across cores, and hpcd does not compete with the scheduler.
It does the opposite: it keeps chosen cores **clean** for one workload.

```console
$ hpcctl shield --unit order-gateway.service --cores 2-5
```

- Cores 2-5 are reserved for the target through cgroup cpusets. Everything else, including
  new processes and movable kernel threads, is kept on housekeeping cores.
- IRQs are steered away from shielded cores, and kept away as devices come and go.
- Like every change, a shield has a snapshot and a lease, and `hpcctl unshield` restores the
  previous layout.
- Conflict rules apply: hpcd will not shield cores that systemd, Kubernetes or another
  cpuset manager already controls.

## Rules

Rules are the heart of hpcd. Each rule describes how to **detect** a problem, **explain** it,
**fix** it and **revert** the fix. They live in [`rules/`](rules/) as TOML files.

```toml
# rules/irq/irq-on-hot-core.toml
id       = "irq-on-hot-core"
title    = "Interrupts handled on a latency-critical core"
category = "irq"
severity = "high"
platform = "linux"
goal     = ["latency", "stability"]   # used by `fix` and by benchmark mode
order    = 10            # fixes with lower order are tried first

[detect]
check        = "irq_affinity_overlaps"
target       = "hot_threads"
min_irq_rate = 1000      # IRQs per second on the hot core
exclude      = ["timer", "LOC"]

[explain]
summary = """
Hardware interrupts preempt whatever is running on the core that handles them.
If that core runs a latency-critical thread, every interrupt adds jitter.
"""
evidence = ["/proc/interrupts", "/proc/irq/*/smp_affinity_list"]

[fix]
action = "set_irq_affinity"
to     = "housekeeping_cores"   # cores with no hot threads, on the same NUMA node when possible
risk   = "low"
reboot = false

[fix.conflicts]
irqbalance = "Ban the affected IRQs or the hot cores in irqbalance, or it will undo this fix"

[references]
links = ["https://docs.kernel.org/core-api/irq/irq-affinity.html"]
```

Rules reference **checks** and **actions** implemented in Rust. A rule decides *when* and
*why*, and tunes the check with parameters and thresholds; the engine guarantees *how safely*.
A rule cannot run arbitrary commands.

This split is deliberate, and it means there are two kinds of contribution:

- **New rule from existing checks.** Combine and tune checks and actions that already exist.
  Pure TOML, no Rust needed.
- **New check or action.** Detection logic or a change the engine cannot express yet. This
  needs Rust and a design discussion in an issue first.

### Initial rule set

| Rule | Category | Runtime fix |
|---|---|---|
| `irq-on-hot-core` | IRQ | Move IRQ affinity to housekeeping cores |
| `deep-cstates` | Power | Hold `/dev/cpu_dma_latency` at 0 |
| `powersave-governor` | Power | `performance` governor on affected cores only |
| `thread-migration` | Scheduler | Pin hot threads with `sched_setaffinity` |
| `smt-sibling-busy` | Scheduler | Move noisy work off the sibling core |
| `remote-numa-memory` | Memory | Report; migrate pages or re-pin to the local node |
| `isolcpus-missing` | Boot | Proposal only (requires reboot) |

### Conflict rules

Other tools manage the same settings. These rules run on every audit and before every fix:

| Rule | Detects | Effect |
|---|---|---|
| `irqbalance-active` | `irqbalance` running | Warns; proposes banned IRQs or cores |
| `tuned-active` | Active `tuned` profile | Reports overlapping settings; won't fight it |
| `systemd-cpuaffinity` | `CPUAffinity=` / `AllowedCPUs=` on the unit | Limits pinning to allowed cores |
| `cpuset-restricted` | cgroup cpuset narrower than the requested cores | Blocks the fix with an explanation |
| `k8s-cpu-manager` | Kubernetes static CPU Manager on the node | Read-only for managed pods |

### Firmware rules

Some of the worst jitter comes from below the operating system. hpcd **detects and proposes
only**: it never writes BIOS or firmware settings.

| Rule | Detects | How |
|---|---|---|
| `smi-activity` | System Management Interrupts freezing all cores | SMI counter (MSR `0x34` on Intel), correlated with latency spikes |
| `bios-power-profile` | Power-saving firmware profile | Frequency and C-state limits, `dmidecode` |
| `bios-cstate-limit` | C-states enabled or capped by firmware | `intel_idle` / ACPI tables vs. driver limits |
| `smt-enabled` | Hyper-threading on latency-critical cores | `/sys/devices/system/cpu/smt` |
| `sub-numa-clustering` | Sub-NUMA Clustering splitting a socket | NUMA topology vs. physical packages |

Each finding comes with vendor-specific instructions (Dell, HPE, Lenovo, Supermicro, and
generic) for the human who will change it. SMI detection needs read access to MSRs, so it
runs through the daemon or as root.

### Writing a rule

1. Copy [`rules/_template.toml`](rules/_template.toml).
2. Use existing checks and actions (see `hpcctl checks` and `hpcctl actions`), or propose a
   new one in an issue.
3. Add a reference that explains *why* the rule matters.
4. Add a test fixture in `rules/tests/` with captured `/proc` and `/sys` data
   (`hpcctl capture` creates one from a live system).
5. Run `cargo test -p hpcd-rules` and open a pull request.

If you have tuned latency-sensitive systems, your knowledge is the most valuable contribution
this project can receive.

## How it works

```
 AI agent (MCP)          human
       │                   │
 ┌─────▼──────┐     ┌──────▼──────┐
 │ hpcctl mcp │     │ hpcctl (CLI)│                          unprivileged
 └─────┬──────┘     └──────┬──────┘
       └─────────┬─────────┘
     Unix socket · bincode · SO_PEERCRED
                 │
        ┌────────▼────────┐
        │      hpcd       │                                  privileged
        │  rules engine   │
        │ policy · leases │
        │ profiles · log  │
        │ watch · shield  │ ──▶ journald · webhook · mcp notifications
        └────────┬────────┘
   /proc · /sys · eBPF · netlink · udev · cpusets · MSRs
```

- **`hpcd`** is the daemon and the only component with privileges.
- **`hpcctl`** is the client: CLI and MCP server (`hpcctl mcp`).
- The daemon verifies the caller with `SO_PEERCRED` and checks every request against
  `policy.toml`.

## Safety model

1. **Read-only by default.** Auditing never changes anything.
2. **AI agents never get privileges.** The MCP server is an unprivileged client.
3. **Snapshots before every change,** persisted to disk.
4. **Leases on every change.** If a fix is not committed before the lease expires, it is
   reverted. On startup the daemon reverts any uncommitted change found in its snapshots, so a
   hung agent, a crashed client or a daemon crash cannot leave the system modified.
5. **One change at a time,** measured against a baseline.
6. **Respect other tools.** Conflict rules run before every fix; hpcd never fights `tuned`,
   `irqbalance`, systemd or cgroup limits silently.
7. **Policy decides** which users can act on which targets and which rules may run in
   automatic mode.
8. **Rules cannot execute commands.** They only combine checks and actions built into the engine.
9. **Boot-time and firmware changes are never applied,** only proposed.
10. **Noise injection is opt-in per host.** `hpcctl noise` refuses to run unless policy
    allows it, and is always leased and reverted.

### Modes

| Mode | Behavior |
|---|---|
| `--dry-run` | Shows what would change |
| `--interactive` | Asks before each fix (default) |
| `--auto` | Applies, measures, keeps or reverts on its own. For benchmark and staging environments |

## MCP tools

Read-only:

| Tool | Returns |
|---|---|
| `system_topology` | Cores, SMT siblings, NUMA nodes, caches |
| `process_profile` | Threads, affinity, context switches, run-queue latency |
| `numa_placement` | Memory placement vs. where threads run |
| `irq_map` | Which IRQs land on which cores |
| `cpu_power_state` | Governors, frequencies, C-states |
| `audit` | Matched rules with evidence and severity |
| `explain_rule` | Full explanation and references for a rule |
| `measure` | Latency histogram and percentiles with confidence intervals |

Actions (require approval or policy):

| Tool | Does |
|---|---|
| `plan_fix` | Proposes a change and its expected effect |
| `apply_fix` | Applies it and opens a lease |
| `compare` | Measures the change against the baseline and returns a verdict with side effects |
| `commit_fix` / `revert_fix` | Keeps or undoes the change |
| `list_changes` | Audit history |
| `watch_events` | Drift, noise and alerts from the watcher (also pushed as notifications) |

```console
$ claude mcp add hpcd -- hpcctl mcp
```

Then ask: *"order-gateway.service has p99 spikes. Find out why and fix what you can safely."*

### Approval

An agent can investigate freely, but it cannot change the system on its own say-so. When it
calls `apply_fix`, the daemon checks `policy.toml`:

- **Allowed by policy** (for example, low-risk rules on a staging host): the fix is applied
  under a lease, exactly as from the CLI.
- **Needs approval** (the default): the daemon holds the request and asks a human, in the
  agent's client or with `hpcctl approve`. The agent sees the request as pending until then.
- **Denied:** the agent gets the reason and can report it.

`commit_fix` always requires a human unless policy says otherwise. Every call, approved or
not, is recorded in the audit log with the agent's request and the human's answer.

## Platform support

hpcd targets **Linux** only (kernel 5.8+ for eBPF measurement). Most of what it tunes is not
exposed on other operating systems.

It works best on **bare metal and dedicated instances**. On shared cloud VMs, the hypervisor
often controls C-states, frequency scaling and interrupt routing, so many fixes will be
reported as unavailable.

## Getting started

> Packages and signed releases are not available yet. Build from source.

Requirements: Linux, Rust stable 1.98+. Latency measurement also needs the nightly toolchain
and `bpf-linker` to build its eBPF program:

```console
$ rustup toolchain install nightly --component rust-src
$ curl -LO https://github.com/aya-rs/bpf-linker/releases/latest/download/bpf-linker-x86_64-unknown-linux-musl.tar.zst
$ mkdir -p ~/.local/bin && tar --zstd -xpf bpf-linker-*.tar.zst -C ~/.local/bin
```

`bpf-linker` is the official prebuilt binary from aya-rs; `cargo install bpf-linker` needs a matching
system LLVM and is not recommended. `~/.local/bin` must be on your `PATH`.

Without them, build with `AYA_BUILD_SKIP=1`: everything works except `hpcctl measure`.

```console
$ git clone https://github.com/angeldevmobile/Daemon
$ cd Daemon
$ cargo build --release
$ cargo install --path crates/hpcctl     # optional: puts hpcctl on your PATH
```

The rule set is built into `hpcctl`. Use `--rules <dir>` (or `HPCD_RULES`) to try rules from
a directory without rebuilding.

Audit without the daemon (read-only, most checks need no root):

```console
$ ./target/release/hpcctl audit --unit <name>.service
```

Measure latency (needs `CAP_BPF` and `CAP_PERFMON`, or root):

```console
$ sudo ./target/release/hpcctl measure --unit <name>.service
```

Run the daemon (needed for fixes):

```console
$ sudo ./target/release/hpcd --config packaging/policy.toml
```

### Try the demo

```console
$ cargo run --release -p jitter-demo -- --noise
$ hpcctl fix --exe target/release/jitter-demo --auto
```

## Project layout

The workspace starts small and grows with the roadmap:

```
hpcd/
├─ crates/
│  ├─ hpcd-core/     shared types: Target, Finding, Plan, Lease, IPC messages
│  ├─ hpcd-probe/    read-only collectors for /proc, /sys and eBPF
│  ├─ hpcd-rules/    rule loader, validation and the check/action registry
│  └─ hpcctl/        client binary: CLI (later: MCP server)
├─ rules/
│  ├─ _template.toml
│  ├─ irq/  power/  sched/  memory/  boot/  conflicts/  firmware/
│  └─ tests/         captured system snapshots for rule tests
├─ bench/jitter-demo/
├─ packaging/        systemd unit, default policy
└─ docs/
```

`hpcd-fix` (actions, snapshots, revert) and `hpcd-daemon` (IPC, policy, leases, profiles)
are added in v0.3.

## Roadmap

Each version is useful on its own.

- [x] **v0.1 — Audit.** `hpcctl audit` and `explain`, rule format, conflict rules, firmware
      rules that need no root, `capture` and offline `audit --snapshot`, fixture tests.
      Read-only, no daemon.
- [ ] **v0.2 — Measure.** eBPF run-queue latency, stability check, confidence intervals,
      guardrail metrics (throughput, CPU, power, memory), self-reported overhead,
      `hpcctl measure --assert`, SMI detection. Read-only MCP tools.
- [ ] **v0.3 — Fix.** Daemon, policy, snapshots, leases, interactive fixes with before/after
      verdicts. MCP action tools. Benchmark mode (`hpcctl run`).
- [ ] **v0.4 — Protect.** Committed profiles per target, re-apply on restart and on driver
      reload, `watch` (drift, noise, alerts, notifications), `shield`, `--auto` mode,
      jitter demo.
- [ ] **v0.5 — Test.** `hpcctl noise` for controlled jitter injection.

**Rule coverage**, added as the engine allows:

- [ ] CPU, IRQs and power — affinity, IRQ routing, governors, C-states
- [ ] Memory — huge pages, THP, NUMA balancing, page placement
- [ ] Network — NIC queues, interrupt coalescing, busy polling, RPS/XPS
- [ ] Timers and I/O — tickless operation, disk schedulers
- [ ] Boot and firmware — SMIs, BIOS power profile, SMT, SNC; detection and proposals only

**Later, not planned yet:** Kubernetes operator, reading firmware settings over Redfish,
`perf`-based hotspot analysis.

**Not planned:** a load balancer of its own (the kernel scheduler does this well), writing
BIOS or firmware settings, a dashboard (hpcd reports to the tools you already use).

### Vision: shared knowledge

Today, what one engineer learns tuning a system is lost. hpcd can change that:

- **Shareable reports.** `hpcctl report --share` produces a Markdown summary (hardware,
  kernel, rules applied, before/after, side effects) ready to paste into an issue, a PR or a
  post.
- **Real-world rule evidence.** Opt-in, anonymized results let each rule show its measured
  effect across machines: *"irq-on-hot-core: −28% median p99 on 340 systems, no effect on
  cloud VMs."* Rules become evidence, not folklore. This needs a careful privacy design
  before any data leaves a machine, and will never be on by default.
- **Rules as a reference.** Every rule already has an explanation, evidence and references.
  A site generated from `rules/`, one page per rule, can become the reference for Linux
  latency tuning, useful even to people who never install hpcd.

## Installing safely

hpcd runs with privileges. Only install it from this repository and its signed releases, and
verify checksums before running any binary.

## Contributing

Rules are the easiest and most valuable way to contribute. See [Writing a rule](#writing-a-rule).
For engine changes, open an issue first to discuss the design. Architecture notes live in `docs/`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
