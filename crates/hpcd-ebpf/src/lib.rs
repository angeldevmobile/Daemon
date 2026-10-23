#![cfg_attr(not(test), no_std)]

pub const BUCKETS: u32 = 16 + 36 * 16;
const MAX_NS: u64 = (1 << 40) - 1;

#[inline(always)]
fn msb(mut v: u64) -> u32 {
    let mut n = 0;
    for shift in [32, 16, 8, 4, 2, 1] {
        if v >= 1 << shift {
            v >>= shift;
            n += shift;
        }
    }
    n
}

#[inline(always)]
pub fn bucket(ns: u64) -> u32 {
    let v = if ns > MAX_NS { MAX_NS } else { ns };
    if v < 16 {
        return v as u32;
    }
    let m = msb(v);
    16 + (m - 4) * 16 + ((v >> (m - 4)) & 15) as u32
}

pub fn bounds(index: u32) -> (u64, u64) {
    if index < 16 {
        return (index as u64, index as u64 + 1);
    }
    let width = 1u64 << ((index - 16) / 16);
    let sub = ((index - 16) % 16) as u64;
    ((16 + sub) * width, (17 + sub) * width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_lands_inside_its_bucket() {
        for ns in (0..5000).chain([65_535, 212_000, 1_310_000, 999_999_999, MAX_NS]) {
            let i = bucket(ns);
            let (lo, hi) = bounds(i);
            assert!(
                i < BUCKETS && lo <= ns && ns < hi,
                "{ns} -> {i} [{lo}, {hi})"
            );
        }
    }

    #[test]
    fn buckets_are_contiguous_and_within_seven_percent() {
        for i in 1..BUCKETS {
            assert_eq!(bounds(i - 1).1, bounds(i).0);
            let (lo, hi) = bounds(i);
            assert!(lo < 16 || (hi - lo) as f64 / lo as f64 <= 1.0 / 16.0 + 1e-9);
        }
        assert_eq!(bucket(u64::MAX), BUCKETS - 1);
    }
}
