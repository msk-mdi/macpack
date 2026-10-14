use std::fmt;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// The type a catalog setting is stored as in its preference domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    String,
}

impl Kind {
    /// The type flag `defaults write` expects for this kind.
    pub fn defaults_flag(self) -> &'static str {
        match self {
            Kind::Bool => "-bool",
            Kind::Int => "-int",
            Kind::Float => "-float",
            Kind::String => "-string",
        }
    }
}

/// A preference value. Serialized untagged so profiles read as plain TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

impl Value {
    /// Parses the text printed by `defaults read` (or typed by a user) as `kind`.
    pub fn parse(kind: Kind, raw: &str) -> Result<Value> {
        let raw = raw.trim();
        Ok(match kind {
            Kind::Bool => match raw.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Value::Bool(true),
                "0" | "false" | "no" | "off" => Value::Bool(false),
                _ => bail!("expected a boolean, got {raw:?}"),
            },
            Kind::Int => match raw.parse::<i64>() {
                Ok(n) => Value::Int(n),
                Err(_) => match raw.parse::<f64>() {
                    Ok(f) if f.is_finite() => Value::Int(f.round() as i64),
                    _ => bail!("expected an integer, got {raw:?}"),
                },
            },
            Kind::Float => match raw.parse::<f64>() {
                Ok(f) if f.is_finite() => Value::Float(f),
                _ => bail!("expected a number, got {raw:?}"),
            },
            Kind::String => Value::String(raw.to_string()),
        })
    }

    /// Checks a value loaded from a profile against the catalog kind,
    /// widening integers to floats where needed.
    pub fn conform(self, kind: Kind) -> Result<Value> {
        Ok(match (kind, self) {
            (Kind::Bool, v @ Value::Bool(_)) => v,
            (Kind::Int, v @ Value::Int(_)) => v,
            (Kind::Float, v @ Value::Float(_)) => v,
            (Kind::Float, Value::Int(n)) => Value::Float(n as f64),
            (Kind::String, v @ Value::String(_)) => v,
            (kind, v) => bail!("expected {kind:?}, got {v}"),
        })
    }

    /// The argument passed to `defaults write` after the type flag.
    pub fn defaults_arg(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(f) => f.to_string(),
            Value::String(s) => s.clone(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => write!(f, "{s:?}"),
            other => f.write_str(&other.defaults_arg()),
        }
    }
}

/// Formats an optional value, where `None` means the key is absent.
pub fn show(value: &Option<Value>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "(unset)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults_output() {
        assert_eq!(Value::parse(Kind::Bool, "1\n").unwrap(), Value::Bool(true));
        assert_eq!(Value::parse(Kind::Bool, "0").unwrap(), Value::Bool(false));
        assert_eq!(Value::parse(Kind::Int, "62").unwrap(), Value::Int(62));
        assert_eq!(Value::parse(Kind::Int, "36.0").unwrap(), Value::Int(36));
        assert_eq!(Value::parse(Kind::Float, "0.5").unwrap(), Value::Float(0.5));
        assert_eq!(Value::parse(Kind::String, "Dark\n").unwrap(), Value::String("Dark".into()));
        assert!(Value::parse(Kind::Bool, "maybe").is_err());
        assert!(Value::parse(Kind::Int, "abc").is_err());
    }

    #[test]
    fn conforms_to_kind() {
        assert_eq!(Value::Int(2).conform(Kind::Float).unwrap(), Value::Float(2.0));
        assert!(Value::String("x".into()).conform(Kind::Bool).is_err());
        assert!(Value::Float(1.5).conform(Kind::Int).is_err());
    }

    #[test]
    fn formats_for_defaults() {
        assert_eq!(Value::Bool(true).defaults_arg(), "true");
        assert_eq!(Value::Float(0.25).defaults_arg(), "0.25");
        assert_eq!(Value::String("png".into()).to_string(), "\"png\"");
        assert_eq!(show(&None), "(unset)");
    }
}
