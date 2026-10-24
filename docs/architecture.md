# Architecture

## Crates

| Crate | Role | Depends on |
|---|---|---|
| `hpcd-core` | Shared types: `Target`, `Finding`, `Severity`, `CpuSet`, `Change`; later `Lease` and IPC messages | — |
| `hpcd-probe` | Read-only collectors. Everything reads through `SysRoot`, so live systems and captures share one code path | `hpcd-core` |
| `hpcd-rules` | Rule loading and validation, checks, registry of checks and actions | `hpcd-core`, `hpcd-probe` |
| `hpcd-fix` | Applies and reverts `Change`s: snapshot, journal on disk, write, verify | `hpcd-core` |
| `hpcctl` | CLI and read-only MCP server (`hpcctl mcp`); later the daemon client | all of the above |
| `jitter-demo` | A periodic loop that shows jitter, for demos and end-to-end tests | — |

Coming in v0.3: `hpcd-daemon` (IPC, policy, leases, profiles).

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
