use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Unit(String),
    Cgroup(PathBuf),
    Exe(PathBuf),
    Pid(u32),
}

impl Target {
    pub fn is_committable(&self) -> bool {
        !matches!(self, Target::Pid(_))
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Unit(u) => write!(f, "{u}"),
            Target::Cgroup(p) | Target::Exe(p) => write!(f, "{}", p.display()),
            Target::Pid(p) => write!(f, "pid {p}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Low,
    #[serde(alias = "med")]
    Medium,
    High,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Info => "INFO",
            Severity::Warn => "WARN",
            Severity::Low => "LOW",
            Severity::Medium => "MED",
            Severity::High => "HIGH",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Goal {
    Latency,
    Stability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl fmt::Display for Risk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub severity: Severity,
    pub message: String,
    pub evidence: Vec<Evidence>,
    pub fixable_at_runtime: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_high_first_when_reversed() {
        let mut s = vec![
            Severity::Info,
            Severity::High,
            Severity::Warn,
            Severity::Medium,
        ];
        s.sort_by(|a, b| b.cmp(a));
        assert_eq!(
            s,
            [
                Severity::High,
                Severity::Medium,
                Severity::Warn,
                Severity::Info
            ]
        );
    }

    #[test]
    fn pid_targets_cannot_be_committed() {
        assert!(!Target::Pid(1).is_committable());
        assert!(Target::Unit("a.service".into()).is_committable());
    }
}
