use hpcd_probe::Load;

use crate::{BLOCK, Histogram, USER_HZ, merge};

const MAX_CPU_POINTS: f64 = 10.0;
const MAX_CPU_SHARE: f64 = 0.2;
const MAX_RATE_CHANGE: f64 = 0.25;
const MIN_RATE_DELTA: f64 = 50.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub p99: Option<u64>,
    pub cpu: f64,
    pub wakeups: f64,
    pub switches: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Wakeups,
    Switches,
    Cpu,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drift {
    pub signal: Signal,
    pub from: f64,
    pub to: f64,
}

pub fn windows(blocks: &[Histogram], load: &[Load], count: usize) -> Vec<Window> {
    let size = blocks.len().min(load.len()) / count.max(1);
    if size == 0 {
        return Vec::new();
    }
    blocks
        .chunks_exact(size)
        .zip(load.chunks_exact(size))
        .take(count)
        .map(|(hists, load)| {
            let secs = size as f64 * BLOCK.as_secs_f64();
            let hist = merge(hists);
            let ticks: u64 = load.iter().map(|l| l.cpu_ticks).sum();
            let switches: u64 = load.iter().map(|l| l.switches).sum();
            Window {
                p99: hist.quantile(0.99),
                cpu: ticks as f64 / USER_HZ / secs * 100.0,
                wakeups: hist.total() as f64 / secs,
                switches: switches as f64 / secs,
            }
        })
        .collect()
}

pub fn drift(windows: &[Window]) -> Vec<Drift> {
    if windows.len() < 2 {
        return Vec::new();
    }
    [Signal::Wakeups, Signal::Switches, Signal::Cpu]
        .into_iter()
        .filter_map(|signal| {
            let values: Vec<f64> = windows
                .iter()
                .map(|w| match signal {
                    Signal::Wakeups => w.wakeups,
                    Signal::Switches => w.switches,
                    Signal::Cpu => w.cpu,
                })
                .collect();
            let by = |cmp: fn(&f64, &f64) -> bool| {
                (0..values.len())
                    .reduce(|best, i| {
                        if cmp(&values[i], &values[best]) {
                            i
                        } else {
                            best
                        }
                    })
                    .unwrap()
            };
            let (lo, hi) = (by(|a, b| a < b), by(|a, b| a > b));
            let (min, max) = (values[lo], values[hi]);
            let spread = max - min;
            let unstable = match signal {
                Signal::Cpu => {
                    let mean = values.iter().sum::<f64>() / values.len() as f64;
                    spread > MAX_CPU_POINTS.max(mean * MAX_CPU_SHARE)
                }
                _ => spread > MIN_RATE_DELTA && spread / min.max(1.0) > MAX_RATE_CHANGE,
            };
            let (from, to) = if lo < hi { (min, max) } else { (max, min) };
            unstable.then_some(Drift { signal, from, to })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(cpu: f64, wakeups: f64, switches: f64) -> Window {
        Window {
            p99: None,
            cpu,
            wakeups,
            switches,
        }
    }

    #[test]
    fn rising_load_is_unstable_like_the_design() {
        let w = [
            window(58.0, 12_100.0, 13_000.0),
            window(83.0, 19_800.0, 13_100.0),
            window(91.0, 24_300.0, 13_050.0),
        ];
        let d = drift(&w);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].signal, Signal::Wakeups);
        assert_eq!((d[0].from, d[0].to), (12_100.0, 24_300.0));
        assert_eq!(d[1].signal, Signal::Cpu);
        assert_eq!((d[1].from, d[1].to), (58.0, 91.0));
    }

    #[test]
    fn small_or_quiet_changes_are_stable() {
        let w = [
            window(61.0, 18_000.0, 20.0),
            window(64.0, 19_500.0, 45.0),
            window(59.0, 18_400.0, 30.0),
        ];
        assert!(drift(&w).is_empty());
        assert!(drift(&w[..1]).is_empty());
    }

    #[test]
    fn many_busy_cores_tolerate_more_cpu_points() {
        let w = [window(700.0, 1000.0, 1000.0), window(760.0, 1000.0, 1000.0)];
        assert!(drift(&w).is_empty());
    }

    #[test]
    fn falling_load_reports_the_drop() {
        let w = [window(50.0, 9000.0, 0.0), window(50.0, 3000.0, 0.0)];
        let d = drift(&w);
        assert_eq!((d[0].from, d[0].to), (9000.0, 3000.0));
    }

    #[test]
    fn windows_split_blocks_and_compute_rates() {
        let mut h = Histogram::default();
        (0..100).for_each(|_| h.record(50_000));
        let blocks = vec![h; 4];
        let load = vec![
            Load {
                cpu_ticks: 50,
                switches: 120,
                ..Default::default()
            };
            4
        ];
        let w = windows(&blocks, &load, 2);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].wakeups, 100.0);
        assert_eq!(w[0].switches, 120.0);
        assert_eq!(w[0].cpu, 50.0);
        assert!(w[0].p99.is_some());
    }
}
