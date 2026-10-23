use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::ProbeError;

#[derive(Debug, Clone)]
pub struct SysRoot {
    root: PathBuf,
}

impl SysRoot {
    pub fn live() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { root: dir.into() }
    }

    pub fn resolve(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    pub fn read(&self, path: &str) -> Result<String, ProbeError> {
        let full = self.resolve(path);
        std::fs::read_to_string(&full).map_err(|source| ProbeError::Io { path: full, source })
    }

    pub fn exists(&self, path: &str) -> bool {
        self.resolve(path).exists()
    }

    pub fn list(&self, path: &str) -> Result<Vec<String>, ProbeError> {
        let full = self.resolve(path);
        let entries = std::fs::read_dir(&full).map_err(|source| ProbeError::Io {
            path: full.clone(),
            source,
        })?;
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, Clone)]
pub enum Source {
    Live { interval: Duration },
    Snapshot { dir: PathBuf, interval: Duration },
}

#[derive(Deserialize)]
struct CaptureMeta {
    interval_ms: u64,
}

impl Source {
    pub fn live(interval: Duration) -> Self {
        Source::Live { interval }
    }

    pub fn snapshot(dir: impl Into<PathBuf>) -> Result<Self, ProbeError> {
        let dir = dir.into();
        let meta_path = dir.join("capture.toml");
        let text = std::fs::read_to_string(&meta_path).map_err(|source| ProbeError::Io {
            path: meta_path.clone(),
            source,
        })?;
        let meta: CaptureMeta = toml::from_str(&text).map_err(|e| ProbeError::Parse {
            path: meta_path,
            message: e.to_string(),
        })?;
        Ok(Source::Snapshot {
            dir,
            interval: Duration::from_millis(meta.interval_ms),
        })
    }

    pub fn interval(&self) -> Duration {
        match self {
            Source::Live { interval } | Source::Snapshot { interval, .. } => *interval,
        }
    }

    pub(crate) fn first(&self) -> SysRoot {
        match self {
            Source::Live { .. } => SysRoot::live(),
            Source::Snapshot { dir, .. } => SysRoot::at(dir.join("t0")),
        }
    }

    pub(crate) fn second(&self) -> SysRoot {
        match self {
            Source::Live { interval } => {
                std::thread::sleep(*interval);
                SysRoot::live()
            }
            Source::Snapshot { dir, .. } => SysRoot::at(dir.join("t1")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_system_paths_under_root() {
        let root = SysRoot::at("fixtures/host-a");
        assert_eq!(
            root.resolve("/proc/interrupts"),
            Path::new("fixtures/host-a").join("proc/interrupts")
        );
    }
}
