use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use hpcd_core::Target;

use crate::{Observation, ProbeError, Source, observe};

pub struct Capture {
    pub observation: Observation,
    pub interval: Duration,
    pub samples: [BTreeMap<String, String>; 2],
}

pub fn capture(source: Source, target: &Target) -> Result<Capture, ProbeError> {
    let source = source.recording();
    let observation = observe(&source, target)?;
    let samples = source.recorded().unwrap_or_default();
    Ok(Capture {
        interval: source.interval(),
        observation,
        samples,
    })
}

impl Capture {
    pub fn files(&self) -> impl Iterator<Item = (&'static str, &str, usize)> {
        ["t0", "t1"]
            .into_iter()
            .zip(&self.samples)
            .flat_map(|(t, files)| files.iter().map(move |(p, c)| (t, p.as_str(), c.len())))
    }

    pub fn write(&self, dir: &Path) -> Result<(), ProbeError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| ProbeError::Io { path, source }
        };
        if dir.exists() && std::fs::read_dir(dir).map_err(io(dir))?.next().is_some() {
            return Err(ProbeError::Target(format!(
                "{} already exists and is not empty",
                dir.display()
            )));
        }
        for (t, files) in ["t0", "t1"].into_iter().zip(&self.samples) {
            for (path, content) in files {
                let dest = dir.join(t).join(path.trim_start_matches('/'));
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(io(parent))?;
                }
                std::fs::write(&dest, content).map_err(io(&dest))?;
            }
        }
        let meta = dir.join("capture.toml");
        let text = format!(
            "interval_ms = {}\ncreated_by = \"hpcctl {}\"\n",
            self.interval.as_millis(),
            env!("CARGO_PKG_VERSION")
        );
        std::fs::write(&meta, text).map_err(io(&meta))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules/tests/order-gateway")
    }

    #[test]
    fn recaptured_fixture_observes_the_same() {
        let target = Target::Unit("order-gateway.service".into());
        let original = observe(&Source::snapshot(fixture()).unwrap(), &target).unwrap();

        let captured = capture(Source::snapshot(fixture()).unwrap(), &target).unwrap();
        assert!(
            captured
                .files()
                .any(|(t, p, _)| t == "t1" && p == "/proc/interrupts")
        );

        let dir = std::env::temp_dir().join(format!("hpcd-capture-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        captured.write(&dir).unwrap();
        let again = observe(&Source::snapshot(&dir).unwrap(), &target).unwrap();
        assert!(captured.write(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(again.pids, original.pids);
        assert_eq!(again.threads.len(), original.threads.len());
        assert_eq!(again.irqs.len(), original.irqs.len());
        assert_eq!(again.cpus, original.cpus);
        assert_eq!(again.processes, original.processes);
    }
}
