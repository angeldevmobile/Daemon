use std::path::{Path, PathBuf};

use hpcd_core::Target;
use hpcd_probe::Source;

fn rules_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules")
}

#[test]
fn irq_on_hot_core_matches_its_fixture() {
    let rules = hpcd_rules::load_dir(&rules_dir()).unwrap();
    let source = Source::snapshot(rules_dir().join("tests/irq-on-hot-core")).unwrap();
    let target = Target::Unit("order-gateway.service".into());
    let obs = hpcd_probe::observe(&source, &target).unwrap();

    assert_eq!(obs.pids, [4821]);
    let hot: Vec<u32> = obs.hot_threads().map(|t| t.tid).collect();
    assert_eq!(hot, [4823]);

    let findings = hpcd_rules::evaluate(&rules, &obs).unwrap();
    let finding = findings
        .iter()
        .find(|f| f.rule_id == "irq-on-hot-core")
        .expect("irq-on-hot-core should match");
    assert_eq!(
        finding.message,
        "IRQs eth0-rx-0, eth0-rx-1, eth0-rx-2 +1 more are handled on core 3 at 8,300/s,\n\
         where hot thread 4823 (gw-io) runs"
    );
    assert_eq!(finding.evidence.len(), 4);
    assert!(finding.evidence[0].excerpt.ends_with("affinity 3"));
    assert!(finding.fixable_at_runtime);
}
