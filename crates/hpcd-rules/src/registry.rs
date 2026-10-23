use hpcd_probe::Observation;

use crate::{Hit, Rule, checks};

pub type CheckFn = fn(&Rule, &Observation) -> Result<Option<Hit>, String>;

pub struct Check {
    pub name: &'static str,
    pub description: &'static str,
    pub run: CheckFn,
}

pub struct Action {
    pub name: &'static str,
    pub description: &'static str,
    pub revert: &'static str,
}

pub const CHECKS: &[Check] = &[Check {
    name: "irq_affinity_overlaps",
    description: "IRQs whose affinity includes cores running hot threads",
    run: checks::irq_affinity_overlaps,
}];

pub const ACTIONS: &[Action] = &[Action {
    name: "set_irq_affinity",
    description: "Set the CPU affinity of IRQs",
    revert: "restore each IRQ's previous smp_affinity_list",
}];

pub fn check(name: &str) -> Option<&'static Check> {
    CHECKS.iter().find(|e| e.name == name)
}

pub fn action(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|e| e.name == name)
}
