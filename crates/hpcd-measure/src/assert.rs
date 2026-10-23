use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    pub label: String,
    pub quantile: f64,
    pub op: Op,
    pub limit_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
}

impl Assertion {
    pub fn holds(&self, value_ns: u64) -> bool {
        match self.op {
            Op::Lt => value_ns < self.limit_ns,
            Op::Le => value_ns <= self.limit_ns,
        }
    }
}

impl FromStr for Assertion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let usage = || format!("'{s}': expected something like \"p99 < 200µs\"");
        let s = s.trim();
        let (left, op, right) = if let Some((l, r)) = s.split_once("<=") {
            (l, Op::Le, r)
        } else if let Some((l, r)) = s.split_once('<') {
            (l, Op::Lt, r)
        } else {
            return Err(usage());
        };
        let label = left.trim().to_lowercase();
        let pct: f64 = label
            .strip_prefix('p')
            .and_then(|p| p.parse().ok())
            .filter(|p| *p > 0.0 && *p < 100.0)
            .ok_or_else(usage)?;
        Ok(Assertion {
            quantile: pct / 100.0,
            label,
            op,
            limit_ns: parse_duration(right.trim()).ok_or_else(usage)?,
        })
    }
}

impl fmt::Display for Assertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self.op {
            Op::Lt => "<",
            Op::Le => "<=",
        };
        write!(
            f,
            "{} {op} {}",
            self.label,
            crate::format_limit(self.limit_ns)
        )
    }
}

fn parse_duration(s: &str) -> Option<u64> {
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let scale = match unit.trim() {
        "ns" => 1.0,
        "us" | "µs" | "μs" => 1e3,
        "ms" => 1e6,
        "s" => 1e9,
        _ => return None,
    };
    let value: f64 = num.parse().ok()?;
    Some((value * scale).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_design_examples() {
        let a: Assertion = "p99 < 200µs".parse().unwrap();
        assert_eq!((a.quantile, a.op, a.limit_ns), (0.99, Op::Lt, 200_000));
        let b: Assertion = "p99.9 < 1ms".parse().unwrap();
        assert_eq!((b.label.as_str(), b.limit_ns), ("p99.9", 1_000_000));
        let c: Assertion = "P50<=20us".parse().unwrap();
        assert_eq!((c.op, c.limit_ns), (Op::Le, 20_000));
    }

    #[test]
    fn rejects_nonsense() {
        for bad in [
            "p99 > 1ms",
            "p100 < 1ms",
            "avg < 1ms",
            "p99 < 1 parsec",
            "p99 <",
        ] {
            assert!(bad.parse::<Assertion>().is_err(), "{bad}");
        }
    }

    #[test]
    fn checks_limits() {
        let a: Assertion = "p99 < 200µs".parse().unwrap();
        assert!(a.holds(182_000) && !a.holds(200_000));
        assert_eq!(a.to_string(), "p99 < 200µs");
    }
}
