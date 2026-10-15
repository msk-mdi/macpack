use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::catalog::{self, Setting};
use crate::defaults::Prefs;
use crate::value::Value;

pub const VERSION: u32 = 1;

/// A portable set of settings, stored as TOML.
#[derive(Debug, Serialize, Deserialize)]
pub struct Profile {
    pub version: u32,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_os: Option<String>,
    #[serde(default)]
    pub settings: Vec<Entry>,
}

/// One setting in a profile. A missing `value` means the key is absent
/// on the source Mac, so applying it restores the system default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

impl Profile {
    pub fn new(entries: impl IntoIterator<Item = (&'static Setting, Option<Value>)>) -> Profile {
        Profile {
            version: VERSION,
            created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            source_os: os_version(),
            settings: entries
                .into_iter()
                .map(|(s, value)| Entry { id: s.id.to_string(), value })
                .collect(),
        }
    }

    /// Reads the current values of `settings`. Settings that can't be read
    /// are left out and reported as warnings.
    pub fn capture(prefs: &dyn Prefs, settings: &[&'static Setting]) -> (Profile, Vec<String>) {
        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        for s in settings {
            match prefs.read(s) {
                Ok(v) => entries.push((*s, v)),
                Err(e) => warnings.push(format!("skipped {}: {e:#}", s.id)),
            }
        }
        (Profile::new(entries), warnings)
    }

    pub fn load(path: &Path) -> Result<Profile> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("can't read {}", path.display()))?;
        let profile: Profile =
            toml::from_str(&text).with_context(|| format!("{} is not a valid profile", path.display()))?;
        if profile.version > VERSION {
            bail!(
                "{} uses profile version {}, this macpack understands up to {VERSION}",
                path.display(),
                profile.version
            );
        }
        Ok(profile)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("can't create {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        let header = "# macpack profile — apply with `macpack apply <this file>`\n";
        std::fs::write(path, format!("{header}{text}"))
            .with_context(|| format!("can't write {}", path.display()))
    }

    /// Maps entries to catalog settings, validating each value. Entries that
    /// are unknown or malformed are returned as warnings and skipped.
    pub fn resolve(&self) -> (Vec<(&'static Setting, Option<Value>)>, Vec<String>) {
        let mut out = Vec::new();
        let mut warnings = Vec::new();
        for entry in &self.settings {
            let Some(setting) = catalog::find(&entry.id) else {
                warnings.push(format!("unknown setting {:?} skipped", entry.id));
                continue;
            };
            if out.iter().any(|(s, _): &(&Setting, _)| s.id == setting.id) {
                warnings.push(format!("duplicate setting {:?} skipped", entry.id));
                continue;
            }
            match entry.value.clone().map(|v| setting.validate(v)).transpose() {
                Ok(v) => out.push((setting, v)),
                Err(e) => warnings.push(format!("{}: {e}", entry.id)),
            }
        }
        (out, warnings)
    }
}

fn os_version() -> Option<String> {
    let out = Command::new("sw_vers").arg("-productVersion").output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Expands a leading `~/` to the home directory.
pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => PathBuf::from(path),
    }
}

/// Default export location: `~/macpack-<computer name>.toml`.
pub fn default_path() -> PathBuf {
    let name = Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "mac".to_string());
    home().join(format!("macpack-{name}.toml"))
}

/// Shortens a path under the home directory to `~/…` for display.
pub fn tilde(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::mem::MemPrefs;

    #[test]
    fn round_trips_through_toml() {
        let prefs = MemPrefs::with(&[
            ("dock.autohide", Value::Bool(true)),
            ("dock.tilesize", Value::Int(48)),
            ("dock.autohide-delay", Value::Float(0.0)),
            ("screenshots.type", Value::String("jpg".into())),
        ]);
        let ids = ["dock.autohide", "dock.tilesize", "dock.autohide-delay", "screenshots.type", "appearance.style"];
        let settings: Vec<_> = ids.iter().map(|id| catalog::find(id).unwrap()).collect();
        let (profile, warnings) = Profile::capture(&prefs, &settings);
        assert!(warnings.is_empty());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/p.toml");
        profile.save(&path).unwrap();
        let loaded = Profile::load(&path).unwrap();
        let (resolved, warnings) = loaded.resolve();
        assert!(warnings.is_empty(), "{warnings:?}");
        let values: Vec<_> = resolved.iter().map(|(s, v)| (s.id, v.clone())).collect();
        assert_eq!(
            values,
            vec![
                ("dock.autohide", Some(Value::Bool(true))),
                ("dock.tilesize", Some(Value::Int(48))),
                ("dock.autohide-delay", Some(Value::Float(0.0))),
                ("screenshots.type", Some(Value::String("jpg".into()))),
                ("appearance.style", None),
            ]
        );
    }

    #[test]
    fn resolve_reports_bad_entries() {
        let profile: Profile = toml::from_str(
            r#"
            version = 1
            created = "now"
            [[settings]]
            id = "dock.autohide"
            value = "yes please"
            [[settings]]
            id = "made.up"
            value = 1
            [[settings]]
            id = "dock.autohide-delay"
            value = 1
            "#,
        )
        .unwrap();
        let (resolved, warnings) = profile.resolve();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].1, Some(Value::Float(1.0)));
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn rejects_newer_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.toml");
        std::fs::write(&path, "version = 99\ncreated = \"x\"\n").unwrap();
        assert!(Profile::load(&path).is_err());
    }
}
