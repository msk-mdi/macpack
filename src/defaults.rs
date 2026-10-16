use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

use crate::catalog::Setting;
use crate::layout;
use crate::value::{Kind, Value};

/// Access to the preference store, so apply logic can be tested without
/// touching the real system.
pub trait Prefs {
    /// Reads a setting; `None` means the key is absent.
    fn read(&self, setting: &Setting) -> Result<Option<Value>>;
    fn write(&self, setting: &Setting, value: &Value) -> Result<()>;
    /// Removes the key so the system default applies. Absent keys are fine.
    fn delete(&self, setting: &Setting) -> Result<()>;
    /// Relaunches a process (Dock, Finder, …) so it picks up new values.
    fn restart(&self, process: &str) -> Result<()>;

    fn set(&self, setting: &Setting, value: Option<&Value>) -> Result<()> {
        match value {
            Some(v) => self.write(setting, v),
            None => self.delete(setting),
        }
    }
}

/// The real store, via the `defaults` command.
pub struct DefaultsCli;

impl DefaultsCli {
    fn run(setting: &Setting, verb: &str, extra: &[String]) -> Result<Output> {
        let mut cmd = Command::new("defaults");
        if setting.host {
            cmd.arg("-currentHost");
        }
        cmd.args([verb, setting.domain, setting.key]).args(extra);
        cmd.output().context("failed to run `defaults`")
    }

    /// Reads a data value. `defaults read` only prints a truncated summary
    /// of data, so this exports the whole domain instead.
    fn read_data(setting: &Setting) -> Result<Option<Vec<u8>>> {
        let mut cmd = Command::new("defaults");
        if setting.host {
            cmd.arg("-currentHost");
        }
        let out = cmd.args(["export", setting.domain, "-"]).output().context("failed to run `defaults`")?;
        if !out.status.success() {
            bail!("defaults export {}: {}", setting.domain, String::from_utf8_lossy(&out.stderr).trim());
        }
        let domain = plist::Value::from_reader_xml(&out.stdout[..])
            .with_context(|| format!("can't parse the {} domain", setting.domain))?;
        match domain.as_dictionary().and_then(|d| d.get(setting.key)) {
            None => Ok(None),
            Some(plist::Value::Data(bytes)) => Ok(Some(bytes.clone())),
            Some(_) => bail!("{} {} is not data", setting.domain, setting.key),
        }
    }
}

fn is_missing(stderr: &str) -> bool {
    stderr.contains("does not exist") || stderr.contains("Could not find") || stderr.contains("not found")
}

impl Prefs for DefaultsCli {
    fn read(&self, setting: &Setting) -> Result<Option<Value>> {
        if setting.kind == Kind::Layout {
            return Self::read_data(setting)?
                .map(|bytes| Ok(Value::String(layout::format(&layout::decode(&bytes)?))))
                .transpose();
        }
        let out = Self::run(setting, "read", &[])?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if is_missing(&stderr) {
                return Ok(None);
            }
            bail!("defaults read {} {}: {}", setting.domain, setting.key, stderr.trim());
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        Value::parse(setting.kind, &stdout).map(Some)
    }

    fn write(&self, setting: &Setting, value: &Value) -> Result<()> {
        let value = setting.validate(value.clone())?;
        let arg = match (setting.kind, &value) {
            (Kind::Layout, Value::String(text)) => {
                layout::encode(&layout::parse(text)?)?.iter().map(|b| format!("{b:02x}")).collect()
            }
            _ => value.defaults_arg(),
        };
        let args = [setting.kind.defaults_flag().to_string(), arg];
        let out = Self::run(setting, "write", &args)?;
        if !out.status.success() {
            bail!(
                "defaults write {} {}: {}",
                setting.domain,
                setting.key,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    fn delete(&self, setting: &Setting) -> Result<()> {
        let out = Self::run(setting, "delete", &[])?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !is_missing(&stderr) {
            bail!("defaults delete {} {}: {}", setting.domain, setting.key, stderr.trim());
        }
        Ok(())
    }

    fn restart(&self, process: &str) -> Result<()> {
        // killall fails when the process isn't running, which is fine.
        Command::new("killall").arg(process).output().context("failed to run `killall`")?;
        Ok(())
    }
}

#[cfg(test)]
pub mod mem {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    /// In-memory store for tests.
    #[derive(Default)]
    pub struct MemPrefs {
        pub values: RefCell<HashMap<String, Value>>,
        pub restarted: RefCell<Vec<String>>,
        /// Ids whose writes fail.
        pub fail_writes: Vec<&'static str>,
    }

    impl MemPrefs {
        pub fn with(values: &[(&str, Value)]) -> Self {
            let me = Self::default();
            for (id, v) in values {
                me.values.borrow_mut().insert(id.to_string(), v.clone());
            }
            me
        }

        pub fn get(&self, id: &str) -> Option<Value> {
            self.values.borrow().get(id).cloned()
        }
    }

    impl Prefs for MemPrefs {
        fn read(&self, setting: &Setting) -> Result<Option<Value>> {
            Ok(self.get(setting.id))
        }

        fn write(&self, setting: &Setting, value: &Value) -> Result<()> {
            if self.fail_writes.contains(&setting.id) {
                bail!("write refused");
            }
            self.values.borrow_mut().insert(setting.id.to_string(), value.clone());
            Ok(())
        }

        fn delete(&self, setting: &Setting) -> Result<()> {
            self.values.borrow_mut().remove(setting.id);
            Ok(())
        }

        fn restart(&self, process: &str) -> Result<()> {
            self.restarted.borrow_mut().push(process.to_string());
            Ok(())
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::value::Kind;

    const DOMAIN: &str = "com.macpack.selftest";

    fn setting(key: &'static str, kind: Kind) -> Setting {
        Setting {
            id: key,
            category: "Test",
            label: key,
            domain: DOMAIN,
            key,
            kind,
            host: false,
            restart: None,
            description: "",
            allowed: crate::catalog::Allowed::Any,
        }
    }

    #[test]
    fn round_trips_through_real_defaults() {
        let cases = [
            (setting("flag", Kind::Bool), Value::Bool(true)),
            (setting("count", Kind::Int), Value::Int(42)),
            (setting("ratio", Kind::Float), Value::Float(0.25)),
            (setting("name", Kind::String), Value::String("hello world".into())),
        ];
        let prefs = DefaultsCli;
        for (s, v) in &cases {
            assert_eq!(prefs.read(s).unwrap(), None);
            prefs.write(s, v).unwrap();
            assert_eq!(prefs.read(s).unwrap().as_ref(), Some(v));
            prefs.delete(s).unwrap();
            assert_eq!(prefs.read(s).unwrap(), None);
            prefs.delete(s).unwrap(); // deleting twice is fine
        }
        let _ = Command::new("defaults").args(["delete", DOMAIN]).output();
        let plist = crate::profile::home().join(format!("Library/Preferences/{DOMAIN}.plist"));
        let _ = std::fs::remove_file(plist);
    }
}
