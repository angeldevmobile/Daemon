mod assert;
#[cfg(all(target_os = "linux", hpcd_ebpf))]
mod ebpf;
mod stats;

use std::time::Duration;

use hpcd_core::Target;

pub use assert::{Assertion, Op};
pub use stats::{Estimate, Histogram, estimate, merge};

pub const BLOCK: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum MeasureError {
    #[error("{0}")]
    Unsupported(String),
    #[error(transparent)]
    Probe(#[from] hpcd_probe::ProbeError),
    #[error("{context}: {message}")]
    Ebpf { context: String, message: String },
}

#[derive(Debug, Clone)]
pub struct Measurement {
    pub pids: Vec<u32>,
    pub threads: usize,
    pub blocks: Vec<Histogram>,
}

impl Measurement {
    pub fn wakeups(&self) -> u64 {
        merge(&self.blocks).total()
    }
}

pub fn measure(
    target: &Target,
    duration: Duration,
    progress: impl FnMut(usize, usize),
) -> Result<Measurement, MeasureError> {
    #[cfg(all(target_os = "linux", hpcd_ebpf))]
    {
        let pids = hpcd_probe::resolve_pids(&hpcd_probe::SysRoot::live(), target)?;
        let blocks = (duration.as_secs_f64() / BLOCK.as_secs_f64())
            .ceil()
            .max(1.0) as usize;
        ebpf::run(pids, blocks, progress)
    }
    #[cfg(not(all(target_os = "linux", hpcd_ebpf)))]
    {
        let _ = (target, duration, progress);
        Err(MeasureError::Unsupported(if cfg!(target_os = "linux") {
            "this hpcctl was built without eBPF support (AYA_BUILD_SKIP); rebuild with nightly and bpf-linker".into()
        } else {
            "measurement needs Linux".into()
        }))
    }
}

fn unit(ns: u64) -> (f64, &'static str, usize) {
    let v = ns as f64;
    match ns {
        0..1_000 => (1.0, "ns", 0),
        1_000..10_000 => (1e3, "µs", 1),
        10_000..1_000_000 => (1e3, "µs", 0),
        1_000_000..10_000_000 => (1e6, "ms", 2),
        10_000_000..100_000_000 => (1e6, "ms", 1),
        _ if v < 1e9 => (1e6, "ms", 0),
        _ => (1e9, "s", 2),
    }
}

pub fn format_ns(ns: u64) -> String {
    let (div, suffix, dec) = unit(ns);
    format!("{:.dec$}{suffix}", ns as f64 / div)
}

pub fn format_limit(ns: u64) -> String {
    let (div, suffix, dec) = unit(ns);
    let text = format!("{:.dec$}", ns as f64 / div);
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    format!("{text}{suffix}")
}

pub fn format_range(low: u64, high: u64, like: u64) -> String {
    let (div, _, dec) = unit(like);
    format!("[{:.dec$}–{:.dec$}]", low as f64 / div, high as f64 / div)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_the_design() {
        assert_eq!(format_ns(212_000), "212µs");
        assert_eq!(format_ns(48_000), "48µs");
        assert_eq!(format_ns(1_310_000), "1.31ms");
        assert_eq!(format_ns(4_200), "4.2µs");
        assert_eq!(format_ns(850), "850ns");
        assert_eq!(format_limit(1_000_000), "1ms");
        assert_eq!(format_limit(200_000), "200µs");
        assert_eq!(format_limit(1_500_000), "1.5ms");
        assert_eq!(format_range(205_000, 219_000, 212_000), "[205–219]");
        assert_eq!(format_range(1_180_000, 1_440_000, 1_310_000), "[1.18–1.44]");
    }
}
