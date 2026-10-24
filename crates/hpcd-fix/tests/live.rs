#![cfg(target_os = "linux")]

use hpcd_core::{Change, CpuSet};
use hpcd_fix::{Fixer, Live, System};

#[test]
fn pins_a_real_thread_and_restores_it() {
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let tid = child.id();
    let sys = Live::new();
    let before = sys.affinity(tid).unwrap();
    let first = before.iter().next().unwrap();
    let dir = std::env::temp_dir().join(format!("hpcd-fix-live-{}", std::process::id()));

    let (mut fixer, _) = Fixer::new(Live::new(), &dir).unwrap();
    let pinned = CpuSet::from_iter([first]);
    let id = fixer
        .apply(&[Change::ThreadAffinity {
            tid,
            cpus: pinned.clone(),
        }])
        .unwrap();
    assert_eq!(sys.affinity(tid).unwrap(), pinned);
    fixer.revert(id).unwrap();
    assert_eq!(sys.affinity(tid).unwrap(), before);

    child.kill().unwrap();
    child.wait().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "needs root"]
fn holds_cpu_dma_latency_while_open() {
    let dir = std::env::temp_dir().join(format!("hpcd-fix-dma-{}", std::process::id()));
    let (mut fixer, _) = Fixer::new(Live::new(), &dir).unwrap();
    let id = fixer.apply(&[Change::CpuDmaLatency { us: 0 }]).unwrap();
    fixer.revert(id).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
