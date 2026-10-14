use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::profile::{self, Profile};

/// `~/Library/Application Support/macpack/backups`
pub fn dir() -> PathBuf {
    profile::home().join("Library/Application Support/macpack/backups")
}

/// Saves `profile` as a timestamped backup in `dir`.
pub fn save(dir: &Path, profile: &Profile) -> Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
    let mut path = dir.join(format!("{stamp}.toml"));
    let mut n = 1;
    while path.exists() {
        n += 1;
        // `~` sorts after `.`, keeping name order equal to creation order.
        path = dir.join(format!("{stamp}~{n}.toml"));
    }
    profile.save(&path)?;
    Ok(path)
}

/// Backups in `dir`, newest first.
pub fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("can't read {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    out.sort();
    out.reverse();
    Ok(out)
}

/// Finds a backup by `latest`, file name (with or without `.toml`) or path.
pub fn find(dir: &Path, name: &str) -> Result<PathBuf> {
    let all = list(dir)?;
    if name == "latest" {
        return all.into_iter().next().context("no backups yet");
    }
    let path = profile::expand(name);
    if path.is_file() {
        return Ok(path);
    }
    let file = if name.ends_with(".toml") { name.to_string() } else { format!("{name}.toml") };
    match all.into_iter().find(|p| p.file_name().is_some_and(|f| f == file.as_str())) {
        Some(p) => Ok(p),
        None => bail!("no backup named {name:?} (see `macpack backups`)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_lists_and_finds() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find(dir.path(), "latest").is_err());
        let a = save(dir.path(), &Profile::new([])).unwrap();
        let b = save(dir.path(), &Profile::new([])).unwrap();
        assert_ne!(a, b);
        assert_eq!(list(dir.path()).unwrap(), vec![b.clone(), a.clone()]);
        assert_eq!(find(dir.path(), "latest").unwrap(), b);
        let stem = a.file_stem().unwrap().to_str().unwrap();
        assert_eq!(find(dir.path(), stem).unwrap(), a);
        assert!(find(dir.path(), "nope").is_err());
    }
}
