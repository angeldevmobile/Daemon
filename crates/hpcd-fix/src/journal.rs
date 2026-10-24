use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::FixError;
use crate::apply::Applied;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub applied: Vec<Applied>,
}

pub struct Journal {
    dir: PathBuf,
}

impl Journal {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, FixError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| FixError::journal(&dir, e))?;
        Ok(Self { dir })
    }

    fn path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id:08}.json"))
    }

    pub fn save(&self, entry: &Entry) -> Result<(), FixError> {
        let path = self.path(entry.id);
        let tmp = path.with_extension("tmp");
        let json = serde_json::to_vec_pretty(entry).map_err(|e| FixError::journal(&path, e))?;
        std::fs::write(&tmp, json).map_err(|e| FixError::journal(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| FixError::journal(&path, e))
    }

    pub fn remove(&self, id: u64) -> Result<(), FixError> {
        let path = self.path(id);
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(FixError::journal(&path, e)),
            _ => Ok(()),
        }
    }

    pub fn pending(&self) -> Result<Vec<Entry>, FixError> {
        let read = std::fs::read_dir(&self.dir).map_err(|e| FixError::journal(&self.dir, e))?;
        let mut entries = Vec::new();
        for item in read {
            let path = item.map_err(|e| FixError::journal(&self.dir, e))?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let text = std::fs::read(&path).map_err(|e| FixError::journal(&path, e))?;
            entries.push(serde_json::from_slice(&text).map_err(|e| FixError::journal(&path, e))?);
        }
        entries.sort_by_key(|e: &Entry| e.id);
        Ok(entries)
    }
}
