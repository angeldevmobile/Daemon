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
        [
            "deep-cstates",
            "irq-on-hot-core",
            "irqbalance-active",
            "isolcpus-missing",
            "powersave-governor",
            "remote-numa-memory",
            "smt-enabled",
            "smt-sibling-busy",
            "thread-migration"
        ]
    );
}

#[test]
fn irqbalance_active() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "irqbalance-active");
    assert_eq!(
        f.message,
        "irqbalance is running and would undo IRQ affinity changes"
    );
    assert_eq!(f.evidence[0].excerpt, "irqbalance (pid 812)");
    assert!(!f.fixable_at_runtime);
}

#[test]
fn isolcpus_missing() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "isolcpus-missing");
    assert_eq!(
        f.message,
        "No isolated cores (requires reboot, proposal only)"
    );
    assert!(!f.fixable_at_runtime);
}

#[test]
fn smt_enabled() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "smt-enabled");
    assert_eq!(f.message, "SMT on · hot core 3 shares a core with cpu 7");
    assert_eq!(f.evidence.len(), 2);
}

#[test]
fn thread_migration() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "thread-migration");
    assert_eq!(
        f.message,
        "Thread 4823 (gw-io) migrated 37 times in 10s (allowed cpus 0-7)"
    );
    assert!(f.fixable_at_runtime);
}

#[test]
fn smt_sibling_busy() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "smt-sibling-busy");
    assert_eq!(
        f.message,
        "Core 7, SMT sibling of hot core 3, is 40% busy with other work"
    );
}

#[test]
fn remote_numa_memory() {
    let findings = audit_order_gateway();
    let f = finding(&findings, "remote-numa-memory");
    assert_eq!(
        f.message,
        "41% of resident memory on node 1, hot thread 4823 runs on node 0"
    );
    assert_eq!(f.evidence[0].excerpt, "N0=590 N1=410 pages");
    assert!(!f.fixable_at_runtime);
}
