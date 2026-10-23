use std::path::Path;

#[test]
fn repository_rules_are_valid() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    let rules = hpcd_rules::load_dir(&dir).unwrap_or_else(|e| panic!("{e}"));
    assert!(rules.iter().any(|r| r.id == "irq-on-hot-core"));
}
