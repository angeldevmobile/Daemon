use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use crate::ProbeError;

pub type Recorder = Arc<Mutex<BTreeMap<String, String>>>;

#[derive(Debug, Clone)]
pub struct SysRoot {
    root: PathBuf,
    recorder: Option<Recorder>,
}

impl SysRoot {
    pub fn live() -> Self {
        Self::at("/")
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self {
            root: dir.into(),
            recorder: None,
        }
    }

    fn recording(mut self, recorder: &Recorder) -> Self {
        self.recorder = Some(recorder.clone());
        self
    }

    pub fn resolve(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    pub fn read(&self, path: &str) -> Result<String, ProbeError> {
        let full = self.resolve(path);
        let text = std::fs::read_to_string(&full)
            .map_err(|source| ProbeError::Io { path: full, source })?;
        self.record(path, &text);
        Ok(text)
    }

    pub fn read_sanitized(
        &self,
        path: &str,
        sanitize: fn(&str) -> String,
    ) -> Result<String, ProbeError> {
        let full = self.resolve(path);
        let text = std::fs::read_to_string(&full)
            .map_err(|source| ProbeError::Io { path: full, source })?;
        self.record(path, &sanitize(&text));
        Ok(text)
    }

    pub fn read_msr(&self, cpu: u32, register: u32) -> Result<u64, ProbeError> {
        let key = format!("/dev/cpu/{cpu}/msr-{register:#x}");
        let value = if self.root == Path::new("/") {
            read_msr_device(&self.resolve(&format!("/dev/cpu/{cpu}/msr")), register)?
        } else {
            let full = self.resolve(&key);
            let text = std::fs::read_to_string(&full).map_err(|source| ProbeError::Io {
                path: full.clone(),
                source,
            })?;
            text.trim().parse().map_err(|_| ProbeError::Parse {
                path: full,
                message: "expected a number".into(),
            })?
        };
        self.record(
            &key,
            &format!(
                "{value}
"
            ),
        );
        Ok(value)
    }

    pub(crate) fn record(&self, path: &str, text: &str) {
        if let Some(recorder) = &self.recorder {
            let key = format!("/{}", path.trim_start_matches('/'));
            if let Ok(mut files) = recorder.lock() {
                files.insert(key, text.to_string());
            }
        }
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

#[cfg(unix)]
fn read_msr_device(path: &Path, register: u32) -> Result<u64, ProbeError> {
    use std::os::unix::fs::FileExt;
    let io = |source| ProbeError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut bytes = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|f| f.read_exact_at(&mut bytes, u64::from(register)))
        .map_err(io)?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(not(unix))]
fn read_msr_device(path: &Path, _register: u32) -> Result<u64, ProbeError> {
    Err(ProbeError::Io {
        path: path.to_path_buf(),
        source: std::io::ErrorKind::Unsupported.into(),
    })
}

#[derive(Debug, Clone)]
pub enum Source {
    Live {
        interval: Duration,
    },
    Snapshot {
        dir: PathBuf,
        interval: Duration,
    },
    Recording {
        inner: Box<Source>,
        samples: [Recorder; 2],
    },
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

    pub fn recording(self) -> Self {
        Source::Recording {
            inner: Box::new(self),
            samples: Default::default(),
        }
    }

    pub fn interval(&self) -> Duration {
        match self {
            Source::Live { interval } | Source::Snapshot { interval, .. } => *interval,
            Source::Recording { inner, .. } => inner.interval(),
        }
    }

    pub(crate) fn first(&self) -> SysRoot {
        match self {
            Source::Live { .. } => SysRoot::live(),
            Source::Snapshot { dir, .. } => SysRoot::at(dir.join("t0")),
            Source::Recording { inner, samples } => inner.first().recording(&samples[0]),
        }
    }

    pub(crate) fn second(&self) -> SysRoot {
        match self {
            Source::Live { interval } => {
                std::thread::sleep(*interval);
                SysRoot::live()
            }
            Source::Snapshot { dir, .. } => SysRoot::at(dir.join("t1")),
            Source::Recording { inner, samples } => inner.second().recording(&samples[1]),
        }
    }

    pub(crate) fn recorded(&self) -> Option<[BTreeMap<String, String>; 2]> {
        let Source::Recording { samples, .. } = self else {
            return None;
        };
        let take = |r: &Recorder| r.lock().map(|m| m.clone()).unwrap_or_default();
        Some([take(&samples[0]), take(&samples[1])])
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
