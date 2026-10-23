use std::collections::BTreeMap;

use hpcd_core::{Goal, Risk, Severity};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Irq,
    Power,
    Sched,
    Memory,
    Boot,
    Conflicts,
    Firmware,
}

impl Category {
    pub fn dir_name(self) -> &'static str {
        match self {
            Category::Irq => "irq",
            Category::Power => "power",
            Category::Sched => "sched",
            Category::Memory => "memory",
            Category::Boot => "boot",
            Category::Conflicts => "conflicts",
            Category::Firmware => "firmware",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub title: String,
    pub category: Category,
    pub severity: Severity,
    pub platform: String,
    #[serde(default)]
    pub goal: Vec<Goal>,
    #[serde(default = "default_order")]
    pub order: u32,
    pub detect: Detect,
    pub explain: Explain,
    pub fix: Option<Fix>,
    #[serde(default)]
    pub propose: BTreeMap<String, Vec<String>>,
    pub references: References,
}

fn default_order() -> u32 {
    100
}

#[derive(Debug, Clone, Deserialize)]
pub struct Detect {
    pub check: String,
    #[serde(flatten)]
    pub params: toml::Table,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Explain {
    pub summary: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fix {
    pub action: String,
    pub risk: Risk,
    #[serde(default)]
    pub reboot: bool,
    #[serde(default)]
    pub conflicts: BTreeMap<String, String>,
    #[serde(flatten)]
    pub params: toml::Table,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct References {
    pub links: Vec<String>,
}
