use std::collections::BTreeMap;

use crate::{CpuSet, SysRoot};

const NODE_DIR: &str = "/sys/devices/system/node";

pub fn read_nodes(root: &SysRoot) -> BTreeMap<u32, CpuSet> {
    root.list(NODE_DIR)
        .unwrap_or_default()
        .iter()
        .filter_map(|name| {
            let node = name.strip_prefix("node")?.parse().ok()?;
            let list = root.read(&format!("{NODE_DIR}/{name}/cpulist")).ok()?;
            Some((node, CpuSet::parse(&list)?))
        })
        .collect()
}

pub fn read_memory(root: &SysRoot, pids: &[u32]) -> BTreeMap<u32, u64> {
    let mut pages = BTreeMap::new();
    for pid in pids {
        let Ok(text) = root.read_sanitized(&format!("/proc/{pid}/numa_maps"), sanitize) else {
            continue;
        };
        for (node, n) in node_pages(&text) {
            *pages.entry(node).or_default() += n;
        }
    }
    pages
}

fn node_pages(text: &str) -> impl Iterator<Item = (u32, u64)> + '_ {
    text.split_whitespace().filter_map(|token| {
        let (node, n) = token.strip_prefix('N')?.split_once('=')?;
        Some((node.parse().ok()?, n.parse().ok()?))
    })
}

fn sanitize(text: &str) -> String {
    text.lines()
        .map(|line| {
            let mut kept: Vec<&str> = line.split_whitespace().take(2).collect();
            kept.extend(line.split_whitespace().filter(|t| {
                t.strip_prefix('N')
                    .and_then(|r| r.split_once('='))
                    .is_some_and(|(n, _)| n.parse::<u32>().is_ok())
            }));
            kept.join(" ") + "\n"
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPS: &str = "\
55d000000000 default file=/opt/secret/app mapped=50 N0=50 kernelpagesize_kB=4
7f0000000000 default anon=950 dirty=950 N0=540 N1=410 kernelpagesize_kB=4
";

    #[test]
    fn sums_pages_per_node() {
        let pages: BTreeMap<u32, u64> = node_pages(MAPS).fold(BTreeMap::new(), |mut m, (k, v)| {
            *m.entry(k).or_default() += v;
            m
        });
        assert_eq!(pages, BTreeMap::from([(0, 590), (1, 410)]));
    }

    #[test]
    fn sanitized_maps_keep_only_node_counts() {
        let clean = sanitize(MAPS);
        assert!(!clean.contains("secret"));
        assert_eq!(
            clean,
            "55d000000000 default N0=50\n7f0000000000 default N0=540 N1=410\n"
        );
    }
}
