use serde::{Deserialize, Serialize};

use crate::{Plan, Target};

pub const DEFAULT_SOCKET: &str = "/run/hpcd/hpcd.sock";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    Apply {
        target: Target,
        rule: String,
        lease_secs: Option<u64>,
    },
    Commit {
        lease: u64,
    },
    Revert {
        lease: u64,
    },
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub id: u64,
    pub target: Target,
    pub plan: Plan,
    pub uid: u32,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum Response {
    Applied { lease: LeaseInfo },
    Committed { lease: u64 },
    Reverted { lease: u64 },
    Status { leases: Vec<LeaseInfo> },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_one_json_object_per_line() {
        let req = Request::Apply {
            target: Target::Pid(42),
            rule: "thread-migration".into(),
            lease_secs: Some(300),
        };
        let line = serde_json::to_string(&req).unwrap();
        assert_eq!(
            line,
            r#"{"request":"apply","target":{"pid":42},"rule":"thread-migration","lease_secs":300}"#
        );
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), req);
    }
}
