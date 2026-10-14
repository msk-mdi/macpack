use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::backup;
use crate::catalog::Setting;
use crate::defaults::Prefs;
use crate::profile::Profile;
use crate::value::Value;

/// A setting whose current value differs from the profile.
#[derive(Debug)]
pub struct Change {
    pub setting: &'static Setting,
    pub current: Option<Value>,
    pub desired: Option<Value>,
    pub selected: bool,
}

/// What applying a profile would do on this Mac.
#[derive(Debug, Default)]
pub struct Plan {
    pub changes: Vec<Change>,
    /// Settings already matching the profile.
    pub unchanged: usize,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn new(profile: &Profile, prefs: &dyn Prefs) -> Plan {
        let (resolved, mut warnings) = profile.resolve();
        let mut changes = Vec::new();
        let mut unchanged = 0;
        for (setting, desired) in resolved {
            match prefs.read(setting) {
                Ok(current) if current == desired => unchanged += 1,
                Ok(current) => changes.push(Change { setting, current, desired, selected: true }),
                Err(e) => warnings.push(format!("skipped {}: {e:#}", setting.id)),
            }
        }
        Plan { changes, unchanged, warnings }
    }

    pub fn selected(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.selected)
    }
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub backup: Option<PathBuf>,
    pub applied: Vec<&'static str>,
    pub errors: Vec<String>,
    pub restarted: Vec<&'static str>,
}

/// Applies the selected changes. The current values are first saved as a
/// backup profile in `backup_dir` (when given), so applying that backup
/// undoes this run. Failed writes are collected rather than aborting.
pub fn apply(plan: &Plan, prefs: &dyn Prefs, backup_dir: Option<&Path>, restart: bool) -> Result<Outcome> {
    let selected: Vec<&Change> = plan.selected().collect();
    let mut outcome = Outcome::default();
    if selected.is_empty() {
        return Ok(outcome);
    }
    if let Some(dir) = backup_dir {
        let previous = Profile::new(selected.iter().map(|c| (c.setting, c.current.clone())));
        outcome.backup = Some(backup::save(dir, &previous)?);
    }
    for change in &selected {
        match prefs.set(change.setting, change.desired.as_ref()) {
            Ok(()) => outcome.applied.push(change.setting.id),
            Err(e) => outcome.errors.push(format!("{}: {e:#}", change.setting.id)),
        }
    }
    if restart {
        for change in &selected {
            let Some(process) = change.setting.restart else { continue };
            if outcome.applied.contains(&change.setting.id) && !outcome.restarted.contains(&process) {
                match prefs.restart(process) {
                    Ok(()) => outcome.restarted.push(process),
                    Err(e) => outcome.errors.push(format!("restart {process}: {e:#}")),
                }
            }
        }
    }
    Ok(outcome)
}

/// Whether any applied setting only takes effect after logout or app relaunch.
pub fn needs_logout(outcome: &Outcome) -> bool {
    outcome
        .applied
        .iter()
        .filter_map(|id| crate::catalog::find(id))
        .any(|s| s.restart.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use crate::defaults::mem::MemPrefs;

    fn profile(entries: &[(&str, Option<Value>)]) -> Profile {
        Profile::new(entries.iter().map(|(id, v)| (catalog::find(id).unwrap(), v.clone())))
    }

    #[test]
    fn plans_only_differences() {
        let prefs = MemPrefs::with(&[
            ("dock.autohide", Value::Bool(true)),
            ("dock.tilesize", Value::Int(48)),
            ("appearance.style", Value::String("Dark".into())),
        ]);
        let p = profile(&[
            ("dock.autohide", Some(Value::Bool(true))),
            ("dock.tilesize", Some(Value::Int(64))),
            ("appearance.style", None),
            ("finder.pathbar", Some(Value::Bool(true))),
        ]);
        let plan = Plan::new(&p, &prefs);
        assert_eq!(plan.unchanged, 1);
        let ids: Vec<_> = plan.changes.iter().map(|c| c.setting.id).collect();
        assert_eq!(ids, ["dock.tilesize", "appearance.style", "finder.pathbar"]);
        assert_eq!(plan.changes[1].current, Some(Value::String("Dark".into())));
    }

    #[test]
    fn applies_backs_up_and_restores() {
        let prefs = MemPrefs::with(&[
            ("dock.tilesize", Value::Int(48)),
            ("appearance.style", Value::String("Dark".into())),
        ]);
        let p = profile(&[
            ("dock.tilesize", Some(Value::Int(64))),
            ("appearance.style", None),
            ("finder.pathbar", Some(Value::Bool(true))),
            ("dock.autohide", Some(Value::Bool(true))),
        ]);
        let mut plan = Plan::new(&p, &prefs);
        // Deselect autohide: it must be neither applied nor backed up.
        plan.changes.iter_mut().find(|c| c.setting.id == "dock.autohide").unwrap().selected = false;

        let dir = tempfile::tempdir().unwrap();
        let out = apply(&plan, &prefs, Some(dir.path()), true).unwrap();
        assert!(out.errors.is_empty());
        assert_eq!(out.applied, ["dock.tilesize", "appearance.style", "finder.pathbar"]);
        assert_eq!(out.restarted, ["Dock", "Finder"]);
        assert_eq!(prefs.get("dock.tilesize"), Some(Value::Int(64)));
        assert_eq!(prefs.get("appearance.style"), None);
        assert_eq!(prefs.get("finder.pathbar"), Some(Value::Bool(true)));
        assert_eq!(prefs.get("dock.autohide"), None);
        assert!(needs_logout(&out));

        // Applying the backup puts everything back.
        let backup = Profile::load(out.backup.as_ref().unwrap()).unwrap();
        assert_eq!(backup.settings.len(), 3);
        let undo = Plan::new(&backup, &prefs);
        apply(&undo, &prefs, None, false).unwrap();
        assert_eq!(prefs.get("dock.tilesize"), Some(Value::Int(48)));
        assert_eq!(prefs.get("appearance.style"), Some(Value::String("Dark".into())));
        assert_eq!(prefs.get("finder.pathbar"), None);
    }

    #[test]
    fn keeps_going_after_failures() {
        let prefs = MemPrefs { fail_writes: vec!["dock.tilesize"], ..Default::default() };
        let p = profile(&[("dock.tilesize", Some(Value::Int(64))), ("finder.pathbar", Some(Value::Bool(true)))]);
        let plan = Plan::new(&p, &prefs);
        let out = apply(&plan, &prefs, None, true).unwrap();
        assert_eq!(out.applied, ["finder.pathbar"]);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.restarted, ["Finder"]);
    }
}
