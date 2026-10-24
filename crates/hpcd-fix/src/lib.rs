mod apply;
mod journal;
mod system;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hpcd_core::Change;

pub use apply::{Applied, Previous};
pub use journal::{Entry, Journal};
pub use system::{Hold, Live, System};

#[derive(Debug, thiserror::Error)]
pub enum FixError {
    #[error("{change}: {source}")]
    Io {
        change: String,
        source: std::io::Error,
    },
    #[error("{change}: the kernel kept {got}")]
    Rejected { change: String, got: String },
    #[error("journal {path}: {message}")]
    Journal { path: PathBuf, message: String },
    #[error("no open change set {0}")]
    Unknown(u64),
}

impl FixError {
    fn journal(path: &Path, e: impl std::fmt::Display) -> Self {
        FixError::Journal {
            path: path.to_path_buf(),
            message: e.to_string(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Recovery {
    pub reverted: Vec<Change>,
    pub failed: Vec<(Change, String)>,
}

struct Open {
    applied: Vec<Applied>,
    holds: Vec<Hold>,
}

pub struct Fixer<S: System> {
    sys: S,
    journal: Journal,
    open: BTreeMap<u64, Open>,
    kept: Vec<Hold>,
    next: u64,
}

fn restore_all(sys: &impl System, applied: &[Applied]) -> Vec<(Change, FixError)> {
    applied
        .iter()
        .rev()
        .filter_map(|a| apply::restore(sys, a).err().map(|e| (a.change.clone(), e)))
        .collect()
}

impl<S: System> Fixer<S> {
    pub fn new(sys: S, journal_dir: impl Into<PathBuf>) -> Result<(Self, Recovery), FixError> {
        let journal = Journal::open(journal_dir)?;
        let mut recovery = Recovery::default();
        let mut next = 1;
        for entry in journal.pending()? {
            next = next.max(entry.id + 1);
            let failed = restore_all(&sys, &entry.applied);
            for a in &entry.applied {
                if !failed.iter().any(|(c, _)| *c == a.change) {
                    recovery.reverted.push(a.change.clone());
                }
            }
            recovery
                .failed
                .extend(failed.into_iter().map(|(c, e)| (c, e.to_string())));
            journal.remove(entry.id)?;
        }
        let fixer = Self {
            sys,
            journal,
            open: BTreeMap::new(),
            kept: Vec::new(),
            next,
        };
        Ok((fixer, recovery))
    }

    pub fn apply(&mut self, changes: &[Change]) -> Result<u64, FixError> {
        let id = self.next;
        self.next += 1;
        let mut entry = Entry {
            id,
            applied: Vec::new(),
        };
        let mut holds = Vec::new();
        for change in changes {
            let step = apply::snapshot(&self.sys, change).and_then(|applied| {
                entry.applied.push(applied);
                self.journal.save(&entry)?;
                apply::write(&self.sys, change)
            });
            match step {
                Ok(hold) => holds.extend(hold),
                Err(e) => {
                    drop(holds);
                    restore_all(&self.sys, &entry.applied);
                    self.journal.remove(id)?;
                    return Err(e);
                }
            }
        }
        self.open.insert(
            id,
            Open {
                applied: entry.applied,
                holds,
            },
        );
        Ok(id)
    }

    pub fn revert(&mut self, id: u64) -> Result<(), FixError> {
        let open = self.open.remove(&id).ok_or(FixError::Unknown(id))?;
        drop(open.holds);
        let failed = restore_all(&self.sys, &open.applied);
        self.journal.remove(id)?;
        match failed.into_iter().next() {
            Some((_, e)) => Err(e),
            None => Ok(()),
        }
    }

    pub fn commit(&mut self, id: u64) -> Result<(), FixError> {
        let open = self.open.remove(&id).ok_or(FixError::Unknown(id))?;
        self.kept.extend(open.holds);
        self.journal.remove(id)
    }

    pub fn open_sets(&self) -> impl Iterator<Item = (u64, &[Applied])> {
        self.open.iter().map(|(id, o)| (*id, o.applied.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io;
    use std::sync::{Arc, Mutex};

    use hpcd_core::CpuSet;

    use super::*;

    #[derive(Clone, Default)]
    struct Fake {
        files: Arc<Mutex<BTreeMap<String, String>>>,
        threads: Arc<Mutex<BTreeMap<u32, CpuSet>>>,
        held: Arc<Mutex<Vec<u32>>>,
        reject: Arc<Mutex<Option<String>>>,
    }

    struct Release(Arc<Mutex<Vec<u32>>>, u32);

    impl Drop for Release {
        fn drop(&mut self) {
            self.0.lock().unwrap().retain(|v| *v != self.1);
        }
    }

    impl Fake {
        fn file(&self, path: &str) -> String {
            self.files.lock().unwrap()[path].clone()
        }
    }

    impl System for Fake {
        fn read(&self, path: &str) -> io::Result<String> {
            self.files
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }

        fn write(&self, path: &str, value: &str) -> io::Result<()> {
            let mut files = self.files.lock().unwrap();
            if !files.contains_key(path) {
                return Err(io::ErrorKind::NotFound.into());
            }
            if self.reject.lock().unwrap().as_deref() != Some(path) {
                files.insert(path.into(), format!("{value}\n"));
            }
            Ok(())
        }

        fn affinity(&self, tid: u32) -> io::Result<CpuSet> {
            self.threads
                .lock()
                .unwrap()
                .get(&tid)
                .cloned()
                .ok_or_else(|| io::Error::from_raw_os_error(3))
        }

        fn set_affinity(&self, tid: u32, cpus: &CpuSet) -> io::Result<()> {
            let mut threads = self.threads.lock().unwrap();
            match threads.get_mut(&tid) {
                Some(c) => {
                    *c = cpus.clone();
                    Ok(())
                }
                None => Err(io::Error::from_raw_os_error(3)),
            }
        }

        fn hold_dma_latency(&self, us: u32) -> io::Result<Hold> {
            self.held.lock().unwrap().push(us);
            Ok(Hold::new(Release(self.held.clone(), us)))
        }
    }

    const IRQ: &str = "/proc/irq/142/smp_affinity_list";
    const GOV: &str = "/sys/devices/system/cpu/cpu3/cpufreq/scaling_governor";

    fn host() -> Fake {
        let fake = Fake::default();
        fake.files.lock().unwrap().extend([
            (IRQ.to_string(), "3\n".to_string()),
            (GOV.to_string(), "powersave\n".to_string()),
        ]);
        fake.threads
            .lock()
            .unwrap()
            .insert(4823, CpuSet::parse("0-7").unwrap());
        fake
    }

    fn journal_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hpcd-fix-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn cpus(list: &str) -> CpuSet {
        CpuSet::parse(list).unwrap()
    }

    fn changes() -> Vec<Change> {
        vec![
            Change::IrqAffinity {
                irq: 142,
                cpus: cpus("0-1"),
            },
            Change::CpuGovernor {
                cpu: 3,
                governor: "performance".into(),
            },
            Change::ThreadAffinity {
                tid: 4823,
                cpus: cpus("3"),
            },
            Change::CpuDmaLatency { us: 0 },
        ]
    }

    #[test]
    fn applies_and_reverts_every_kind_of_change() {
        let fake = host();
        let dir = journal_dir("revert");
        let (mut fixer, _) = Fixer::new(fake.clone(), &dir).unwrap();
        let id = fixer.apply(&changes()).unwrap();
        assert_eq!(fake.file(IRQ), "0-1\n");
        assert_eq!(fake.file(GOV), "performance\n");
        assert_eq!(fake.threads.lock().unwrap()[&4823], cpus("3"));
        assert_eq!(*fake.held.lock().unwrap(), [0]);
        assert_eq!(Journal::open(&dir).unwrap().pending().unwrap().len(), 1);

        fixer.revert(id).unwrap();
        assert_eq!(fake.file(IRQ), "3\n");
        assert_eq!(fake.file(GOV), "powersave\n");
        assert_eq!(fake.threads.lock().unwrap()[&4823], cpus("0-7"));
        assert!(fake.held.lock().unwrap().is_empty());
        assert!(Journal::open(&dir).unwrap().pending().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rolls_back_when_the_kernel_rejects_a_change() {
        let fake = host();
        *fake.reject.lock().unwrap() = Some(GOV.into());
        let dir = journal_dir("reject");
        let (mut fixer, _) = Fixer::new(fake.clone(), &dir).unwrap();
        let err = fixer.apply(&changes()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "cpu 3 governor → performance: the kernel kept powersave"
        );
        assert_eq!(fake.file(IRQ), "3\n");
        assert_eq!(fake.threads.lock().unwrap()[&4823], cpus("0-7"));
        assert!(Journal::open(&dir).unwrap().pending().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reverts_what_a_crashed_run_left_behind() {
        let fake = host();
        let dir = journal_dir("crash");
        {
            let (mut fixer, _) = Fixer::new(fake.clone(), &dir).unwrap();
            fixer.apply(&changes()).unwrap();
            std::mem::forget(fixer);
        }
        fake.threads.lock().unwrap().clear();
        let (_, recovery) = Fixer::new(fake.clone(), &dir).unwrap();
        assert_eq!(fake.file(IRQ), "3\n");
        assert_eq!(fake.file(GOV), "powersave\n");
        assert_eq!(recovery.reverted.len(), 4);
        assert!(recovery.failed.is_empty());
        assert!(Journal::open(&dir).unwrap().pending().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn commit_keeps_the_change_and_forgets_the_snapshot() {
        let fake = host();
        let dir = journal_dir("commit");
        let (mut fixer, _) = Fixer::new(fake.clone(), &dir).unwrap();
        let id = fixer.apply(&changes()).unwrap();
        fixer.commit(id).unwrap();
        assert_eq!(fake.file(GOV), "performance\n");
        assert_eq!(*fake.held.lock().unwrap(), [0]);
        assert!(Journal::open(&dir).unwrap().pending().unwrap().is_empty());
        assert!(matches!(fixer.revert(id), Err(FixError::Unknown(_))));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
