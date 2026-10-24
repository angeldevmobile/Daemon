use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use hpcd_core::Target;
use hpcd_probe::{CpuSet, Observation, Source};
use serde_json::{Value, json};

use crate::{audit_report, inspect, load_rules, measure_report, render};

const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct Server {
    pub rules: Option<PathBuf>,
    pub snapshot: Option<PathBuf>,
    pub sample: Duration,
}

pub fn serve(server: Server) -> Result<()> {
    if running_as_root() {
        bail!("hpcctl mcp refuses to run as root: AI agents never get privileges");
    }
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle(&line) {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn running_as_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            let uids = s.lines().find_map(|l| l.strip_prefix("Uid:"))?.to_string();
            uids.split_whitespace().nth(1).map(|euid| euid == "0")
        })
        .unwrap_or(false)
}

fn reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn text(text: &str, is_error: bool) -> Value {
    let plain = anstream::adapter::strip_str(text).to_string();
    json!({"content": [{"type": "text", "text": plain}], "isError": is_error})
}

impl Server {
    pub fn handle(&self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return Some(error(Value::Null, -32700, &e.to_string())),
        };
        let id = msg.get("id").cloned()?;
        let method = msg["method"].as_str().unwrap_or_default();
        let params = &msg["params"];
        Some(match method {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or_default();
                let version = PROTOCOLS
                    .iter()
                    .find(|v| **v == asked)
                    .unwrap_or(&PROTOCOLS[0]);
                reply(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "hpcd", "version": env!("CARGO_PKG_VERSION")},
                        "instructions": "Read-only tools to find out why a Linux workload has latency spikes. \
                            Start with audit, then look closer with irq_map, cpu_power_state, \
                            process_profile or numa_placement. Nothing here changes the system.",
                    }),
                )
            }
            "ping" => reply(id, json!({})),
            "tools/list" => reply(id, json!({"tools": tools()})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                if !TOOLS.iter().any(|t| t.0 == name) {
                    return Some(error(id, -32602, &format!("unknown tool: {name}")));
                }
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call(name, &args) {
                    Ok(out) => reply(id, text(&out, false)),
                    Err(e) => reply(id, text(&format!("{e:#}"), true)),
                }
            }
            _ => error(id, -32601, &format!("method not found: {method}")),
        })
    }

    fn source(&self) -> Result<Source> {
        match &self.snapshot {
            Some(dir) => {
                Source::snapshot(dir).with_context(|| format!("opening capture {}", dir.display()))
            }
            None if !cfg!(target_os = "linux") => {
                bail!("live data needs Linux; start the server with hpcctl mcp --snapshot <dir>")
            }
            None => Ok(Source::live(self.sample)),
        }
    }

    fn host(&self) -> Result<Observation> {
        Ok(hpcd_probe::observe_host(&self.source()?)?)
    }

    fn target(&self, args: &Value) -> Result<(Target, Observation)> {
        let target = target(args)?;
        let obs = hpcd_probe::observe(&self.source()?, &target)?;
        Ok((target, obs))
    }

    fn call(&self, name: &str, args: &Value) -> Result<String> {
        match name {
            "system_topology" => Ok(inspect::topology(&self.host()?)),
            "process_profile" => {
                let (target, obs) = self.target(args)?;
                Ok(inspect::process(&obs, &target.to_string()))
            }
            "numa_placement" => {
                let (target, obs) = self.target(args)?;
                Ok(inspect::numa(&obs, &target.to_string()))
            }
            "irq_map" => Ok(inspect::irqs(&self.host()?, cores(args)?.as_ref())),
            "cpu_power_state" => Ok(inspect::power(&self.host()?, cores(args)?.as_ref())),
            "audit" => audit_report(
                self.rules.as_deref(),
                &target(args)?,
                self.source()?,
                args["firmware"].as_bool().unwrap_or(false),
            ),
            "explain_rule" => {
                let id = args["rule"].as_str().context("missing 'rule'")?;
                let rules = load_rules(self.rules.as_deref())?;
                let rule = rules
                    .iter()
                    .find(|r| r.id == id)
                    .with_context(|| format!("no rule named '{id}'"))?;
                Ok(render::explain(rule))
            }
            "measure" => self.measure(args),
            _ => Err(anyhow!("unknown tool: {name}")),
        }
    }

    fn measure(&self, args: &Value) -> Result<String> {
        if self.snapshot.is_some() {
            bail!("measure needs the live system, not a capture");
        }
        let number = |key: &str, default: u64| args[key].as_u64().unwrap_or(default);
        let asserts: Vec<String> = args["assert"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let wait_stable = match args["wait_stable"].as_str() {
            Some(s) => Some(hpcd_core::parse_duration(s).map_err(anyhow::Error::msg)?),
            None => None,
        };
        let (text, code) = measure_report(
            target(args)?,
            number("windows", 5) as u32,
            number("window", 4),
            wait_stable,
            &asserts,
            false,
        )
        .map_err(|e| {
            e.context(
                "measure needs CAP_BPF and CAP_PERFMON, and hpcctl mcp never runs with \
                 privileges; it will go through the hpcd daemon from v0.3",
            )
        })?;
        let verdict = match code {
            0 => "verdict: pass (exit 0)",
            1 => "verdict: a latency budget was exceeded (exit 1)",
            _ => "verdict: none, the load was not stable (exit 2)",
        };
        Ok(format!("{text}\n\n{verdict}"))
    }
}

fn target(args: &Value) -> Result<Target> {
    let mut found = Vec::new();
    if let Some(u) = args["unit"].as_str() {
        found.push(Target::Unit(u.into()));
    }
    if let Some(p) = args["pid"].as_u64() {
        found.push(Target::Pid(u32::try_from(p).context("pid out of range")?));
    }
    if let Some(c) = args["cgroup"].as_str() {
        found.push(Target::Cgroup(c.into()));
    }
    if let Some(e) = args["exe"].as_str() {
        found.push(Target::Exe(e.into()));
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        _ => bail!("give exactly one of unit, pid, cgroup or exe"),
    }
}

fn cores(args: &Value) -> Result<Option<CpuSet>> {
    args["cores"]
        .as_str()
        .map(|s| CpuSet::parse(s).with_context(|| format!("'{s}': expected a cpu list like 2-5")))
        .transpose()
}

const TOOLS: [(&str, &str); 8] = [
    (
        "system_topology",
        "Cores, sockets, SMT siblings, NUMA nodes, isolated cpus and caches.",
    ),
    (
        "process_profile",
        "Threads of a workload: CPU use, core, allowed cpus, context switches and migrations per second.",
    ),
    (
        "numa_placement",
        "Where a workload's memory lives per NUMA node versus where its hot threads run.",
    ),
    (
        "irq_map",
        "Which IRQs land on which cores, with rates and affinity. Optional cores filter, e.g. \"3\" or \"2-5\".",
    ),
    (
        "cpu_power_state",
        "Frequency governor, current and allowed frequency, and enabled C-states with exit latency, grouped by cpus.",
    ),
    (
        "audit",
        "Run the hpcd rules against a workload: matched rules with severity, message and evidence. firmware=true audits BIOS and firmware settings instead.",
    ),
    (
        "explain_rule",
        "Full explanation, evidence sources, fix and references for one rule id from audit.",
    ),
    (
        "measure",
        "Run-queue latency percentiles (p50, p99, p99.9) with 95% confidence intervals, a load stability check, guardrail metrics and hpcd's own overhead. Takes windows × window seconds.",
    ),
];

fn tools() -> Vec<Value> {
    let target = json!({
        "unit": {"type": "string", "description": "systemd unit, e.g. order-gateway.service"},
        "pid": {"type": "integer", "description": "process id"},
        "cgroup": {"type": "string", "description": "cgroup directory"},
        "exe": {"type": "string", "description": "absolute path of the executable"},
    });
    let cores = json!({"cores": {"type": "string", "description": "cpu list, e.g. 3 or 2-5"}});
    let with = |base: &Value, extra: Value| {
        let mut props = base.as_object().cloned().unwrap_or_default();
        props.extend(extra.as_object().cloned().unwrap_or_default());
        Value::Object(props)
    };
    TOOLS
        .iter()
        .map(|(name, description)| {
            let properties = match *name {
                "system_topology" => json!({}),
                "irq_map" | "cpu_power_state" => cores.clone(),
                "audit" => with(
                    &target,
                    json!({"firmware": {"type": "boolean", "description": "audit firmware and BIOS settings"}}),
                ),
                "explain_rule" => {
                    json!({"rule": {"type": "string", "description": "rule id, e.g. irq-on-hot-core"}})
                }
                "measure" => with(
                    &target,
                    json!({
                        "windows": {"type": "integer", "description": "number of windows (default 5)"},
                        "window": {"type": "integer", "description": "seconds per window (default 4)"},
                        "assert": {"type": "array", "items": {"type": "string"}, "description": "conditions like \"p99 < 200µs\""},
                        "wait_stable": {"type": "string", "description": "keep measuring until the load is steady, at most this long, e.g. 2m"},
                    }),
                ),
                _ => target.clone(),
            };
            let mut schema = json!({"type": "object", "properties": properties});
            if *name == "explain_rule" {
                schema["required"] = json!(["rule"]);
            }
            json!({
                "name": name,
                "description": description,
                "inputSchema": schema,
                "annotations": {"readOnlyHint": true, "openWorldHint": false},
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn server() -> Server {
        Server {
            rules: None,
            snapshot: Some(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules/tests/order-gateway"),
            ),
            sample: Duration::from_secs(3),
        }
    }

    fn call(s: &Server, name: &str, args: Value) -> Value {
        let msg = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                         "params": {"name": name, "arguments": args}});
        s.handle(&msg.to_string()).unwrap()
    }

    #[test]
    fn negotiates_and_lists_read_only_tools() {
        let s = server();
        let init = s
            .handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#)
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(
            s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
        let list = s
            .handle(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 8);
        assert!(
            tools
                .iter()
                .all(|t| t["annotations"]["readOnlyHint"] == true)
        );
    }

    #[test]
    fn audits_a_capture() {
        let s = server();
        let out = call(&s, "audit", json!({"unit": "order-gateway.service"}));
        let text = out["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(out["result"]["isError"], false);
        assert!(text.contains("irq-on-hot-core"), "{text}");
        assert!(!text.contains('\x1b'), "{text}");
    }

    #[test]
    fn reports_bad_input_as_tool_errors() {
        let s = server();
        let out = call(&s, "audit", json!({}));
        assert_eq!(out["result"]["isError"], true);
        let out = call(&s, "measure", json!({"pid": 1}));
        assert_eq!(out["result"]["isError"], true);
        let out = call(&s, "nope", json!({}));
        assert_eq!(out["error"]["code"], -32602);
        let out = s.handle("{not json").unwrap();
        assert_eq!(out["error"]["code"], -32700);
    }
}
