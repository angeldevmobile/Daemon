use std::path::{Path, PathBuf};

use hpcd_core::{Finding, Target};
use hpcd_probe::Source;

fn rules_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules")
}

fn audit_order_gateway() -> Vec<Finding> {
    let rules = hpcd_rules::load_dir(&rules_dir()).unwrap();
    let source = Source::snapshot(rules_dir().join("tests/order-gateway")).unwrap();
    let target = Target::Unit("order-gateway.service".into());
    let obs = hpcd_probe::observe(&source, &target).unwrap();

    assert_eq!(obs.pids, [4821]);
    let hot: Vec<u32> = obs.hot_threads().map(|t| t.tid).collect();
    assert_eq!(hot, [4823]);

    hpcd_rules::evaluate(&rules, &obs).unwrap()
}

fn finding<'a>(findings: &'a [Finding], id: &str) -> &'a Finding {
    findings
        .iter()
        .find(|f| f.rule_id == id)
        .unwrap_or_else(|| panic!("{id} should match"))
}

#[test]
fn irq_on_hot_core() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "irq-on-hot-core");
    assert_eq!(
        f.message,
        "IRQs eth0-rx-0, eth0-rx-1, eth0-rx-2 +1 more are handled on core 3 at 8,300/s,\n\
         where hot thread 4823 (gw-io) runs"
    );
    assert_eq!(f.evidence.len(), 4);
    assert!(f.evidence[0].excerpt.ends_with("affinity 3"));
    assert!(f.fixable_at_runtime);
}

#[test]
fn deep_cstates() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "deep-cstates");
    assert_eq!(f.message, "C6 enabled on core 3 (exit latency 133µs)");
    assert_eq!(f.evidence.len(), 1);
    assert!(f.fixable_at_runtime);
}

#[test]
fn powersave_governor() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "powersave-governor");
    assert_eq!(f.message, "Governor 'powersave' on core 3");
    assert_eq!(f.evidence[0].excerpt, "powersave · driver intel_pstate");
}

#[test]
fn only_expected_rules_match() {
    let mut ids: Vec<String> = audit_order_gateway()
        .into_iter()
        .map(|f| f.rule_id)
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        ["deep-cstates", "irq-on-hot-core", "powersave-governor"]
    );
}
