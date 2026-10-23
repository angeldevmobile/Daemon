use std::path::Path;

#[test]
fn repository_rules_are_valid() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    let rules = hpcd_rules::load_dir(&dir).unwrap_or_else(|e| panic!("{e}"));
    assert!(rules.iter().any(|r| r.id == "irq-on-hot-core"));
}

#[test]
fn embedded_rules_match_the_rules_directory() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    let ids = |rules: Vec<hpcd_rules::Rule>| rules.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(
        ids(hpcd_rules::load_embedded().unwrap()),
        ids(hpcd_rules::load_dir(&dir).unwrap())
    );
}
