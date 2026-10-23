use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrqLine {
    pub label: String,
    pub counts: BTreeMap<u32, u64>,
    pub description: String,
    pub name: String,
}

impl IrqLine {
    pub fn number(&self) -> Option<u32> {
        self.label.parse().ok()
    }
}

pub fn parse(text: &str) -> Option<Vec<IrqLine>> {
    let mut lines = text.lines();
    let cpus: Vec<u32> = lines
        .next()?
        .split_whitespace()
        .map(|c| c.strip_prefix("CPU")?.parse().ok())
        .collect::<Option<_>>()?;

    let mut irqs = Vec::new();
    for line in lines {
        let Some((label, rest)) = line.split_once(':') else {
            continue;
        };
        let mut tokens = rest.split_whitespace().peekable();
        let mut counts = BTreeMap::new();
        for &cpu in &cpus {
            match tokens.peek().and_then(|t| t.parse::<u64>().ok()) {
                Some(n) => {
                    counts.insert(cpu, n);
                    tokens.next();
                }
                None => break,
            }
        }
        let description: Vec<&str> = tokens.collect();
        let label = label.trim().to_string();
        irqs.push(IrqLine {
            name: description.last().map_or(label.clone(), |n| n.to_string()),
            description: description.join(" "),
            label,
            counts,
        });
    }
    Some(irqs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
           CPU0       CPU1       CPU3
  0:         36          0          0   IO-APIC   2-edge      timer
142:          0          5       2140   IR-PCI-MSI 524288-edge      eth0-rx-0
LOC:        100        200        300   Local timer interrupts
ERR:          0
";

    #[test]
    fn parses_counts_per_online_cpu() {
        let irqs = parse(SAMPLE).unwrap();
        assert_eq!(irqs.len(), 4);
        let eth = &irqs[1];
        assert_eq!(eth.number(), Some(142));
        assert_eq!(eth.name, "eth0-rx-0");
        assert_eq!(eth.counts[&3], 2140);
        assert_eq!(irqs[2].label, "LOC");
        assert_eq!(irqs[2].number(), None);
        assert_eq!(irqs[3].counts.len(), 1);
    }
}
