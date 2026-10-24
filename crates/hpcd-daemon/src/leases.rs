use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use hpcd_core::ipc::LeaseInfo;
use hpcd_core::{Change, Plan, Target};
use hpcd_fix::{Fixer, System};

pub struct Lease {
    pub target: Target,
    pub plan: Plan,
    pub uid: u32,
    pub expires: Instant,
}

pub struct Leases<S: System> {
    fixer: Fixer<S>,
    open: BTreeMap<u64, Lease>,
}

fn resource(change: &Change) -> String {
    match change {
        Change::IrqAffinity { irq, .. } => format!("IRQ {irq}"),
        Change::CpuGovernor { cpu, .. } => format!("the governor of cpu {cpu}"),
        Change::ThreadAffinity { tid, .. } => format!("thread {tid}"),
        Change::CpuDmaLatency { .. } => "/dev/cpu_dma_latency".into(),
    }
}

fn may_act(lease: &Lease, uid: u32) -> Result<(), String> {
    if uid == 0 || uid == lease.uid {
        Ok(())
    } else {
        Err(format!("lease belongs to uid {}", lease.uid))
    }
}

impl<S: System> Leases<S> {
    pub fn new(fixer: Fixer<S>) -> Self {
        Self {
            fixer,
            open: BTreeMap::new(),
        }
    }

    fn info(id: u64, lease: &Lease, now: Instant) -> LeaseInfo {
        LeaseInfo {
            id,
            target: lease.target.clone(),
            plan: lease.plan.clone(),
            uid: lease.uid,
            expires_in_secs: lease.expires.saturating_duration_since(now).as_secs(),
        }
    }

    pub fn apply(
        &mut self,
        target: Target,
        plan: Plan,
        uid: u32,
        duration: Duration,
        now: Instant,
    ) -> Result<LeaseInfo, String> {
        if plan.reboot {
            return Err(format!(
                "{} needs a reboot; hpcd only proposes it",
                plan.rule_id
            ));
        }
        if plan.changes.is_empty() {
            let why = plan.note.clone().unwrap_or_else(|| "no changes".into());
            return Err(format!("{} has nothing to apply yet: {why}", plan.rule_id));
        }
        for change in &plan.changes {
            let taken = self.open.iter().find(|(_, l)| {
                l.plan
                    .changes
                    .iter()
                    .any(|c| resource(c) == resource(change))
            });
            if let Some((id, _)) = taken {
                return Err(format!(
                    "{} is already changed by lease {id}; commit or revert it first",
                    resource(change)
                ));
            }
        }
        let id = self.fixer.apply(&plan.changes).map_err(|e| e.to_string())?;
        let lease = Lease {
            target,
            plan,
            uid,
            expires: now + duration,
        };
        let info = Self::info(id, &lease, now);
        self.open.insert(id, lease);
        Ok(info)
    }

    pub fn commit(&mut self, id: u64, uid: u32) -> Result<(), String> {
        let lease = self.open.get(&id).ok_or(format!("no open lease {id}"))?;
        may_act(lease, uid)?;
        if !lease.target.is_committable() {
            return Err(format!(
                "{} is a one-off target and cannot be committed; use --unit, --cgroup or --exe",
                lease.target
            ));
        }
        self.fixer.commit(id).map_err(|e| e.to_string())?;
        self.open.remove(&id);
        Ok(())
    }

    pub fn revert(&mut self, id: u64, uid: u32) -> Result<(), String> {
        let lease = self.open.get(&id).ok_or(format!("no open lease {id}"))?;
        may_act(lease, uid)?;
        self.open.remove(&id);
        self.fixer.revert(id).map_err(|e| e.to_string())
    }

    pub fn expire(&mut self, now: Instant) -> Vec<(u64, Result<(), String>)> {
        let due: Vec<u64> = self
            .open
            .iter()
            .filter(|(_, l)| l.expires <= now)
            .map(|(id, _)| *id)
            .collect();
        due.into_iter()
            .map(|id| {
                self.open.remove(&id);
                (id, self.fixer.revert(id).map_err(|e| e.to_string()))
            })
            .collect()
    }

    pub fn revert_all(&mut self) -> Vec<(u64, Result<(), String>)> {
        let ids: Vec<u64> = self.open.keys().rev().copied().collect();
        ids.into_iter()
            .map(|id| {
                self.open.remove(&id);
                (id, self.fixer.revert(id).map_err(|e| e.to_string()))
            })
            .collect()
    }

    pub fn next_expiry(&self) -> Option<Instant> {
        self.open.values().map(|l| l.expires).min()
    }

    pub fn status(&self, now: Instant) -> Vec<LeaseInfo> {
        self.open
            .iter()
            .map(|(id, l)| Self::info(*id, l, now))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io;
    use std::sync::{Arc, Mutex};

    use hpcd_core::{CpuSet, Risk};
    use hpcd_fix::Hold;

    use super::*;

    #[derive(Clone, Default)]
    struct Threads(Arc<Mutex<BTreeMap<u32, CpuSet>>>);

    impl System for Threads {
        fn read(&self, _: &str) -> io::Result<String> {
            Err(io::ErrorKind::NotFound.into())
        }
        fn write(&self, _: &str, _: &str) -> io::Result<()> {
            Err(io::ErrorKind::NotFound.into())
        }
        fn affinity(&self, tid: u32) -> io::Result<CpuSet> {
            Ok(self.0.lock().unwrap()[&tid].clone())
        }
        fn set_affinity(&self, tid: u32, cpus: &CpuSet) -> io::Result<()> {
            self.0.lock().unwrap().insert(tid, cpus.clone());
            Ok(())
        }
        fn hold_dma_latency(&self, _: u32) -> io::Result<Hold> {
            Ok(Hold::new(()))
        }
    }

    fn pin(tid: u32, cpu: u32) -> Plan {
        Plan {
            rule_id: "thread-migration".into(),
            summary: format!("pin thread {tid} to core {cpu}"),
            changes: vec![Change::ThreadAffinity {
                tid,
                cpus: CpuSet::from_iter([cpu]),
            }],
            risk: Risk::Medium,
            reboot: false,
            conflicts: vec![],
            note: None,
        }
    }

    fn setup(name: &str) -> (Threads, Leases<Threads>, std::path::PathBuf) {
        let threads = Threads::default();
        threads.0.lock().unwrap().extend([
            (10, CpuSet::parse("0-7").unwrap()),
            (11, CpuSet::parse("0-7").unwrap()),
        ]);
        let dir = std::env::temp_dir().join(format!("hpcd-leases-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (fixer, _) = Fixer::new(threads.clone(), &dir).unwrap();
        (threads, Leases::new(fixer), dir)
    }

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn expired_leases_are_reverted() {
        let (threads, mut leases, dir) = setup("expire");
        let t0 = Instant::now();
        let a = leases
            .apply(Target::Pid(10), pin(10, 3), 1000, MIN, t0)
            .unwrap();
        leases
            .apply(Target::Pid(11), pin(11, 4), 1000, 5 * MIN, t0)
            .unwrap();
        assert_eq!(a.expires_in_secs, 60);
        assert_eq!(leases.next_expiry(), Some(t0 + MIN));
        assert_eq!(threads.0.lock().unwrap()[&10].to_string(), "3");

        let expired = leases.expire(t0 + 2 * MIN);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, a.id);
        assert_eq!(threads.0.lock().unwrap()[&10].to_string(), "0-7");
        assert_eq!(threads.0.lock().unwrap()[&11].to_string(), "4");
        assert_eq!(leases.status(t0 + 2 * MIN)[0].expires_in_secs, 180);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_the_owner_or_root_may_act_and_pids_cannot_be_committed() {
        let (_, mut leases, dir) = setup("owner");
        let now = Instant::now();
        let id = leases
            .apply(Target::Pid(10), pin(10, 3), 1000, MIN, now)
            .unwrap()
            .id;
        assert_eq!(
            leases.revert(id, 1001).unwrap_err(),
            "lease belongs to uid 1000"
        );
        assert!(
            leases
                .commit(id, 1000)
                .unwrap_err()
                .contains("cannot be committed")
        );
        leases.revert(id, 0).unwrap();
        assert!(leases.status(now).is_empty());

        let unit = Target::Unit("gw.service".into());
        let id = leases.apply(unit, pin(10, 3), 1000, MIN, now).unwrap().id;
        leases.commit(id, 1000).unwrap();
        assert!(leases.next_expiry().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_overlapping_and_non_runtime_plans() {
        let (_, mut leases, dir) = setup("overlap");
        let now = Instant::now();
        let id = leases
            .apply(Target::Pid(10), pin(10, 3), 1000, MIN, now)
            .unwrap()
            .id;
        let err = leases
            .apply(Target::Pid(10), pin(10, 5), 1000, MIN, now)
            .unwrap_err();
        assert_eq!(
            err,
            format!("thread 10 is already changed by lease {id}; commit or revert it first")
        );
        let mut reboot = pin(11, 1);
        reboot.reboot = true;
        assert!(
            leases
                .apply(Target::Pid(11), reboot, 1000, MIN, now)
                .is_err()
        );
        let mut empty = pin(11, 1);
        empty.changes.clear();
        empty.note = Some("tasks are listed when applied".into());
        assert_eq!(
            leases
                .apply(Target::Pid(11), empty, 1000, MIN, now)
                .unwrap_err(),
            "thread-migration has nothing to apply yet: tasks are listed when applied"
        );
        assert_eq!(leases.revert_all().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
