use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::registry;
use crate::rule::{Category, Rule};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_rules.rs"));
}

pub fn load_embedded() -> Result<Vec<Rule>, LoadError> {
    let rules = embedded::RULES
        .iter()
        .map(|(path, text)| parse(Path::new(path), text).map(|r| (PathBuf::from(path), r)))
        .collect::<Result<_, _>>()?;
    finish(rules)
}

pub fn load_dir(dir: &Path) -> Result<Vec<Rule>, LoadError> {
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    let rules = files
        .into_iter()
        .map(|path| load_file(&path).map(|r| (path, r)))
        .collect::<Result<_, _>>()?;
    finish(rules)
}

pub fn load_file(path: &Path) -> Result<Rule, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.into(),
        source,
    })?;
    parse(path, &text)
}

fn parse(path: &Path, text: &str) -> Result<Rule, LoadError> {
    let rule: Rule = toml::from_str(text).map_err(|source| LoadError::Parse {
        path: path.into(),
        source: Box::new(source),
    })?;
    validate(path, &rule)?;
    Ok(rule)
}

fn finish(loaded: Vec<(PathBuf, Rule)>) -> Result<Vec<Rule>, LoadError> {
    let mut seen = HashSet::new();
    let mut rules = Vec::with_capacity(loaded.len());
    for (path, rule) in loaded {
        if !seen.insert(rule.id.clone()) {
            return Err(invalid(&path, format!("duplicate rule id '{}'", rule.id)));
        }
        rules.push(rule);
    }
    rules.sort_by_key(|r| r.order);
    Ok(rules)
}

fn validate(path: &Path, rule: &Rule) -> Result<(), LoadError> {
    if path.file_stem().and_then(|s| s.to_str()) != Some(rule.id.as_str()) {
        return Err(invalid(
            path,
            format!("file name must match id '{}'", rule.id),
        ));
    }
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str());
    if parent != Some(rule.category.dir_name()) {
        return Err(invalid(
            path,
            format!(
                "category '{}' must live in rules/{0}/",
                rule.category.dir_name()
            ),
        ));
    }
    if registry::check(&rule.detect.check).is_none() {
        return Err(invalid(
            path,
            format!("unknown check '{}'", rule.detect.check),
        ));
    }
    if let Some(fix) = &rule.fix {
        if registry::action(&fix.action).is_none() {
            return Err(invalid(path, format!("unknown action '{}'", fix.action)));
        }
    }
    if rule.references.links.is_empty() {
        return Err(invalid(path, "at least one reference is required".into()));
    }
    if rule.category == Category::Firmware && !rule.propose.contains_key("generic") {
        return Err(invalid(
            path,
            "firmware rules need a [propose] generic entry".into(),
        ));
    }
    Ok(())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), LoadError> {
    let entries = std::fs::read_dir(dir).map_err(|source| LoadError::Io {
        path: dir.into(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| LoadError::Io {
            path: dir.into(),
            source,
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if name != "tests" {
                collect(&path, out)?;
            }
        } else if path.extension().is_some_and(|e| e == "toml") {
            out.push(path);
        }
    }
    Ok(())
}

fn invalid(path: &Path, message: String) -> LoadError {
    LoadError::Invalid {
        path: path.into(),
        message,
    }
}
