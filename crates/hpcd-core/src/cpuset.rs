use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuSet(BTreeSet<u32>);

impl CpuSet {
    pub fn parse(list: &str) -> Option<Self> {
        let mut cpus = BTreeSet::new();
        for part in list.trim().split(',').filter(|p| !p.is_empty()) {
            match part.split_once('-') {
                Some((a, b)) => cpus.extend(a.parse::<u32>().ok()?..=b.parse::<u32>().ok()?),
                None => {
                    cpus.insert(part.parse().ok()?);
                }
            }
        }
        Some(Self(cpus))
    }

    pub fn contains(&self, cpu: u32) -> bool {
        self.0.contains(&cpu)
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<u32> for CpuSet {
    fn from_iter<I: IntoIterator<Item = u32>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl fmt::Display for CpuSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut cpus = self.iter().peekable();
        let mut first = true;
        while let Some(start) = cpus.next() {
            let mut end = start;
            while cpus.peek() == Some(&(end + 1)) {
                end = cpus.next().unwrap_or(end);
            }
            if !first {
                f.write_str(",")?;
            }
            first = false;
            if start == end {
                write!(f, "{start}")?;
            } else {
                write!(f, "{start}-{end}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_kernel_cpu_lists() {
        let set = CpuSet::parse("0-3,5,7-8\n").unwrap();
        assert!(set.contains(2) && set.contains(5) && !set.contains(6));
        assert_eq!(set.to_string(), "0-3,5,7-8");
    }

    #[test]
    fn rejects_garbage() {
        assert!(CpuSet::parse("a-3").is_none());
    }
}
