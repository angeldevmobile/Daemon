use hpcd_ebpf::{BUCKETS, bounds, bucket};

const RESAMPLES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    counts: Vec<u64>,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            counts: vec![0; BUCKETS as usize],
        }
    }
}

impl Histogram {
    pub fn from_counts(mut counts: Vec<u64>) -> Self {
        counts.resize(BUCKETS as usize, 0);
        Self { counts }
    }

    pub fn record(&mut self, ns: u64) {
        self.counts[bucket(ns) as usize] += 1;
    }

    pub fn add(&mut self, other: &Histogram) {
        for (a, b) in self.counts.iter_mut().zip(&other.counts) {
            *a += b;
        }
    }

    pub fn since(&self, earlier: &Histogram) -> Histogram {
        Histogram {
            counts: self
                .counts
                .iter()
                .zip(&earlier.counts)
                .map(|(a, b)| a.saturating_sub(*b))
                .collect(),
        }
    }

    pub fn total(&self) -> u64 {
        self.counts.iter().sum()
    }

    pub fn quantile(&self, q: f64) -> Option<u64> {
        let total = self.total();
        if total == 0 {
            return None;
        }
        let rank = ((q * total as f64).ceil() as u64).clamp(1, total);
        let mut seen = 0;
        for (i, n) in self.counts.iter().enumerate() {
            seen += n;
            if seen >= rank {
                let (lo, hi) = bounds(i as u32);
                return Some(lo + (hi - lo) / 2);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    pub value: u64,
    pub low: u64,
    pub high: u64,
}

pub fn merge(blocks: &[Histogram]) -> Histogram {
    let mut all = Histogram::default();
    for b in blocks {
        all.add(b);
    }
    all
}

pub fn estimate(blocks: &[Histogram], q: f64) -> Option<Estimate> {
    let value = merge(blocks).quantile(q)?;
    let mut rng = SplitMix(0x6870_6364);
    let mut samples: Vec<u64> = (0..RESAMPLES)
        .filter_map(|_| {
            let mut h = Histogram::default();
            for _ in 0..blocks.len() {
                h.add(&blocks[rng.below(blocks.len())]);
            }
            h.quantile(q)
        })
        .collect();
    samples.sort_unstable();
    let at = |p: f64| samples[((p * (samples.len() - 1) as f64).round()) as usize];
    Some(Estimate {
        value,
        low: at(0.025).min(value),
        high: at(0.975).max(value),
    })
}

struct SplitMix(u64);

impl SplitMix {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(values: impl IntoIterator<Item = u64>) -> Histogram {
        let mut h = Histogram::default();
        values.into_iter().for_each(|v| h.record(v));
        h
    }

    #[test]
    fn quantiles_are_within_bucket_precision() {
        let h = block((1..=10_000).map(|i| i * 100));
        let p99 = h.quantile(0.99).unwrap() as f64;
        assert!((p99 / 990_000.0 - 1.0).abs() < 0.035, "{p99}");
        assert_eq!(Histogram::default().quantile(0.99), None);
    }

    #[test]
    fn interval_contains_the_estimate_and_widens_with_noise() {
        let steady: Vec<_> = (0..50)
            .map(|_| block((1..=1000).map(|i| i * 200)))
            .collect();
        let noisy: Vec<_> = (0..50)
            .map(|b| block((1..=1000).map(|i| i * if b % 5 == 0 { 600 } else { 200 })))
            .collect();
        let s = estimate(&steady, 0.99).unwrap();
        let n = estimate(&noisy, 0.99).unwrap();
        assert!(s.low <= s.value && s.value <= s.high);
        assert!(n.high - n.low > s.high - s.low);
    }

    #[test]
    fn since_subtracts_cumulative_snapshots() {
        let earlier = block([100, 200]);
        let later = block([100, 200, 300, 300]);
        assert_eq!(later.since(&earlier), block([300, 300]));
    }
}
