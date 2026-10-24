use std::time::Duration;

use hpcd_core::parse_duration;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub leases: LeaseLimits,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Access {
    pub groups: Vec<String>,
}

impl Default for Access {
    fn default() -> Self {
        Self {
            groups: vec!["hpcd".into()],
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseLimits {
    #[serde(deserialize_with = "duration")]
    pub default: Duration,
    #[serde(deserialize_with = "duration")]
    pub max: Duration,
}

impl Default for LeaseLimits {
    fn default() -> Self {
        Self {
            default: Duration::from_secs(300),
            max: Duration::from_secs(3600),
        }
    }
}

fn duration<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    let text = String::deserialize(d)?;
    parse_duration(&text).map_err(serde::de::Error::custom)
}

impl Policy {
    pub fn parse(text: &str) -> Result<Self, String> {
        let policy: Policy = toml::from_str(text).map_err(|e| e.message().to_string())?;
        if policy.leases.default > policy.leases.max {
            return Err("leases.default is longer than leases.max".into());
        }
        Ok(policy)
    }

    pub fn lease(&self, asked: Option<u64>) -> Result<Duration, String> {
        let Some(secs) = asked else {
            return Ok(self.leases.default);
        };
        let asked = Duration::from_secs(secs);
        if asked.is_zero() || asked > self.leases.max {
            return Err(format!(
                "lease must be between 1s and {}s (leases.max in policy.toml)",
                self.leases.max.as_secs()
            ));
        }
        Ok(asked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_packaged_policy() {
        let text = include_str!("../../../packaging/policy.toml");
        let policy = Policy::parse(text).unwrap();
        assert_eq!(policy.access.groups, ["hpcd"]);
        assert_eq!(policy.leases.default, Duration::from_secs(300));
        assert_eq!(policy.lease(None).unwrap(), Duration::from_secs(300));
        assert_eq!(policy.lease(Some(60)).unwrap(), Duration::from_secs(60));
        assert!(policy.lease(Some(7200)).is_err());
    }

    #[test]
    fn rejects_bad_lease_limits() {
        let text = "[leases]\ndefault = \"2h\"\nmax = \"1h\"\n";
        assert!(Policy::parse(text).is_err());
        assert!(Policy::parse("[leases]\ndefault = \"soon\"\nmax = \"1h\"\n").is_err());
    }
}
