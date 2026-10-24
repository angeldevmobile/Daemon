use std::time::Duration;

use crate::USER_HZ;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EbpfCost {
    pub run_time: Duration,
    pub runs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overhead {
    pub elapsed: Duration,
    pub cpu_ticks: u64,
    pub rss_kib: u64,
    pub ebpf: Option<EbpfCost>,
}

impl Overhead {
    pub fn cpu(&self) -> f64 {
        let ebpf = self.ebpf.map_or(0.0, |e| e.run_time.as_secs_f64());
        let busy = self.cpu_ticks as f64 / USER_HZ + ebpf;
        busy / self.elapsed.as_secs_f64().max(f64::EPSILON) * 100.0
    }

    pub fn per_event(&self) -> Option<Duration> {
        self.ebpf
            .filter(|e| e.runs > 0)
            .map(|e| e.run_time / e.runs.min(u32::MAX as u64) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_userspace_and_ebpf_time() {
        let o = Overhead {
            elapsed: Duration::from_secs(100),
            cpu_ticks: 30,
            rss_kib: 14_336,
            ebpf: Some(EbpfCost {
                run_time: Duration::from_millis(400),
                runs: 4_000_000,
            }),
        };
        assert!((o.cpu() - 0.7).abs() < 1e-9);
        assert_eq!(o.per_event(), Some(Duration::from_nanos(100)));
        let unknown = Overhead { ebpf: None, ..o };
        assert!((unknown.cpu() - 0.3).abs() < 1e-9);
        assert_eq!(unknown.per_event(), None);
    }
}
