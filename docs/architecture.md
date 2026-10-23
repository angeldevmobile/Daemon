# Architecture

## Crates

| Crate | Role | Depends on |
|---|---|---|
| `hpcd-core` | Shared types: `Target`, `Finding`, `Severity`, later `Plan`, `Lease`, IPC messages | — |
| `hpcd-probe` | Read-only collectors. Everything reads through `SysRoot`, so live systems and captures share one code path | `hpcd-core` |
| `hpcd-rules` | Rule loading and validation, checks, registry of checks and actions | `hpcd-core`, `hpcd-probe` |
| `hpcctl` | CLI; later the MCP server (`hpcctl mcp`) and the daemon client | all of the above |
| `jitter-demo` | A periodic loop that shows jitter, for demos and end-to-end tests | — |

Added in v0.3: `hpcd-fix` (actions, snapshots, revert) and `hpcd-daemon` (IPC, policy,
leases, profiles).

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
Rule fixtures in `rules/tests/<rule-id>/` use the same format.
