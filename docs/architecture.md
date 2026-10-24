# Architecture

## Crates

| Crate | Role | Depends on |
|---|---|---|
| `hpcd-core` | Shared types: `Target`, `Finding`, `Severity`, `CpuSet`, `Change`; later `Lease` and IPC messages | — |
| `hpcd-probe` | Read-only collectors. Everything reads through `SysRoot`, so live systems and captures share one code path | `hpcd-core` |
| `hpcd-rules` | Rule loading and validation, checks, registry of checks and actions | `hpcd-core`, `hpcd-probe` |
| `hpcd-fix` | Applies and reverts `Change`s: snapshot, journal on disk, write, verify | `hpcd-core` |
| `hpcd-daemon` | The `hpcd` binary: Unix socket, `SO_PEERCRED`, `policy.toml`, leases. The only program that links `hpcd-fix` | `hpcd-core`, `hpcd-probe`, `hpcd-rules`, `hpcd-fix` |
| `hpcctl` | CLI and read-only MCP server (`hpcctl mcp`); later the daemon client | all of the above |
| `jitter-demo` | A periodic loop that shows jitter, for demos and end-to-end tests | — |

## Daemon protocol

`hpcctl` connects to `/run/hpcd/hpcd.sock` and sends one JSON object per line
(`hpcd_core::ipc::Request`); the daemon answers with one line (`Response`). The daemon reads
the caller's uid with `SO_PEERCRED`. Root, the daemon's own user and members of the groups in
`[access]` of `policy.toml` may apply, commit and revert; anyone may read the status.

A client asks for a rule on a target, never for raw changes: the daemon observes the system,
evaluates the rule and plans the changes itself. Every applied plan is a lease. A lease that is
not committed is reverted when it expires, when the daemon stops, or, after a crash, when it
starts again. Two open leases never touch the same IRQ, governor, thread or
`/dev/cpu_dma_latency`.

## Applying a change

`hpcd-fix` applies a set of changes all or nothing. For each change it reads the current
value, saves it to the journal (one JSON file per set, written with rename so it is never
half-written), writes the new value and reads it back. If the kernel keeps another value, or
any step fails, the changes already made are restored and the journal entry is removed.
On start, any journal entry left by a crash is restored before anything else happens.

## Boundaries

- **Rules never execute anything.** A rule names a check and an action from the registry
  and passes parameters. Unknown names fail validation at load time.
- **Probing is pure reading.** `hpcd-probe` never writes; writes live only in `hpcd-fix`,
  which only the daemon links.
- **Testability.** Rule tests run against snapshots in `rules/tests/`, so they work on any
  OS, including CI runners without root.

## Development on Windows

The workspace builds and its tests run on any OS. Running against a live system needs Linux;
use WSL or a Linux VM.

## Capture format

A capture is a directory with two samples of the same files, taken `interval_ms` apart:

```
capture/
├─ capture.toml      interval_ms = 10000
├─ t0/proc/...       first sample
└─ t1/proc/...       second sample, plus static files (/proc/irq/*/smp_affinity_list)
```

Rates (IRQs per second, CPU share per thread) come from the difference between `t0` and `t1`.
Rule fixtures in `rules/tests/<host>/` use the same format; one fixture can exercise several
rules (`rules/tests/order-gateway/` covers the eight rules of the audit screen).
