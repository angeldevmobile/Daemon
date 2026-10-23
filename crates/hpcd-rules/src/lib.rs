mod checks;
mod loader;
pub mod registry;
mod rule;

use hpcd_core::Finding;
use hpcd_probe::Observation;

pub use checks::Hit;
pub use loader::{LoadError, load_dir, load_embedded, load_file};
pub use rule::{Category, Detect, Explain, Fix, References, Rule};

#[derive(Debug, thiserror::Error)]
#[error("rule '{rule}': {message}")]
pub struct EvalError {
    pub rule: String,
    pub message: String,
}

pub fn evaluate(rules: &[Rule], obs: &Observation) -> Result<Vec<Finding>, EvalError> {
    let mut findings = Vec::new();
    for rule in rules {
        let error = |message: String| EvalError {
            rule: rule.id.clone(),
            message,
        };
        let check = registry::check(&rule.detect.check)
            .ok_or_else(|| error(format!("unknown check '{}'", rule.detect.check)))?;
        if let Some(hit) = (check.run)(rule, obs).map_err(error)? {
            findings.push(Finding {
                rule_id: rule.id.clone(),
                severity: rule.severity,
                message: hit.message,
                evidence: hit.evidence,
                fixable_at_runtime: rule.fix.as_ref().is_some_and(|f| !f.reboot),
            });
        }
    }
    Ok(findings)
}
