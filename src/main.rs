mod apply;
mod backup;
mod catalog;
mod defaults;
mod layout;
mod profile;
mod tui;
mod value;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;

use crate::apply::Plan;
use crate::catalog::CATALOG;
use crate::defaults::{DefaultsCli, Prefs};
use crate::profile::Profile;
use crate::value::Value;

/// Export macOS settings to a portable profile and apply it on another Mac.
///
/// Run without a command to open the interactive interface.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List known settings and their current values on this Mac.
    List {
        /// Only show this category (e.g. Dock, Finder).
        #[arg(short, long)]
        category: Option<String>,
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Print the current value of one setting (nothing when unset).
    Get {
        /// Setting id (see `macpack list`).
        id: String,
    },
    /// Change one setting on this Mac, backing up its current value first.
    Set {
        /// Setting id (see `macpack list`).
        id: String,
        /// New value, e.g. true, 48, left.
        #[arg(required_unless_present = "unset", allow_negative_numbers = true)]
        value: Option<String>,
        /// Remove the key so the system default applies.
        #[arg(long, conflicts_with = "value")]
        unset: bool,
        /// Only print what would change.
        #[arg(long)]
        dry_run: bool,
        /// Don't restart Dock, Finder, SystemUIServer or ControlCenter afterwards.
        #[arg(long)]
        no_restart: bool,
    },
    /// Save settings to a profile. Opens the picker unless --all, --ids or --changed is given.
    Export {
        /// Profile file to write [default: ~/macpack-<computer>.toml].
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Export every known setting.
        #[arg(long, conflicts_with = "ids")]
        all: bool,
        /// Comma-separated setting ids or categories to export (see `macpack list`).
        #[arg(long, value_delimiter = ',')]
        ids: Vec<String>,
        /// Leave out settings that are unset (still at the system default).
        #[arg(long)]
        changed: bool,
    },
    /// Apply a profile. Opens the review screen unless --dry-run or --yes is given.
    Apply {
        profile: PathBuf,
        /// Only print what would change.
        #[arg(long)]
        dry_run: bool,
        /// Apply every change without asking.
        #[arg(short, long, conflicts_with = "dry_run")]
        yes: bool,
        /// Don't restart Dock, Finder, SystemUIServer or ControlCenter afterwards.
        #[arg(long)]
        no_restart: bool,
    },
    /// Check whether this Mac matches a profile. Exits with status 1 if it doesn't.
    Check {
        profile: PathBuf,
        /// Print nothing; only set the exit status.
        #[arg(short, long)]
        quiet: bool,
    },
    /// List backups made before each apply.
    Backups {
        /// Delete all but the newest KEEP backups.
        #[arg(long, value_name = "KEEP")]
        prune: Option<usize>,
    },
    /// Undo an apply by re-applying a backup.
    Restore {
        /// Backup name, path, or `latest`.
        #[arg(default_value = "latest")]
        backup: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(short, long, conflicts_with = "dry_run")]
        yes: bool,
        #[arg(long)]
        no_restart: bool,
    },
    /// Print a shell completion script, e.g. `macpack completions zsh`.
    Completions { shell: clap_complete::Shell },
    /// Print the man page.
    #[command(hide = true)]
    Man,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let prefs = DefaultsCli;
    match cli.command {
        None => interactive(&prefs, tui::Start::Home)?,
        Some(Command::List { category, json }) => list(&prefs, category.as_deref(), json)?,
        Some(Command::Get { id }) => {
            if let Some(v) = prefs.read(find(&id)?)? {
                println!("{}", v.defaults_arg());
            }
        }
        Some(Command::Set { id, value, unset, dry_run, no_restart }) => {
            let setting = find(&id)?;
            let value = match value {
                Some(raw) if !unset => Some(setting.validate(Value::parse(setting.kind, &raw)?)?),
                _ => None,
            };
            let profile = Profile::new([(setting, value)]);
            apply_profile(&prefs, &profile, setting.id, dry_run, !no_restart)?;
        }
        Some(Command::Export { output, all, ids, changed }) => {
            if !all && ids.is_empty() && !changed {
                interactive(&prefs, tui::Start::Export(output))?;
            } else {
                export(&prefs, output.unwrap_or_else(profile::default_path), &ids, changed)?;
            }
        }
        Some(Command::Apply { profile, dry_run, yes, no_restart }) => {
            if !dry_run && !yes {
                Profile::load(&profile)?; // fail fast before opening the TUI
                interactive(&prefs, tui::Start::Apply(profile))?;
            } else {
                apply_cli(&prefs, &profile, dry_run, !no_restart)?;
            }
        }
        Some(Command::Check { profile, quiet }) => {
            let plan = Plan::new(&Profile::load(&profile)?, &prefs);
            if !quiet {
                print_plan(&plan, &profile.display().to_string());
            }
            if !plan.changes.is_empty() {
                return Ok(ExitCode::FAILURE);
            }
        }
        Some(Command::Backups { prune }) => {
            let dir = backup::dir();
            if let Some(keep) = prune {
                let removed = backup::prune(&dir, keep)?;
                println!("Deleted {} backup(s), kept the newest {keep}.", removed.len());
                return Ok(ExitCode::SUCCESS);
            }
            let list = backup::list(&dir)?;
            if list.is_empty() {
                println!("No backups yet in {}", profile::tilde(&dir));
            }
            for p in list {
                let n = Profile::load(&p).map(|pr| pr.settings.len().to_string()).unwrap_or_else(|_| "?".into());
                println!("{:<28} {n:>3} settings", p.file_stem().unwrap_or_default().to_string_lossy());
            }
        }
        Some(Command::Restore { backup: name, dry_run, yes, no_restart }) => {
            let path = backup::find(&backup::dir(), &name)?;
            if !dry_run && !yes {
                interactive(&prefs, tui::Start::Apply(path))?;
            } else {
                apply_cli(&prefs, &path, dry_run, !no_restart)?;
            }
        }
        Some(Command::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "macpack", &mut std::io::stdout());
        }
        Some(Command::Man) => clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?,
    }
    Ok(ExitCode::SUCCESS)
}

fn find(id: &str) -> Result<&'static catalog::Setting> {
    catalog::find(id).with_context(|| format!("unknown setting {id:?} (see `macpack list`)"))
}

fn interactive(prefs: &dyn Prefs, start: tui::Start) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        bail!("the interactive interface needs a terminal; see `macpack --help` for non-interactive commands");
    }
    tui::run(prefs, start)
}

/// One row of `macpack list --json`.
#[derive(Serialize)]
struct ListRow {
    id: &'static str,
    category: &'static str,
    label: &'static str,
    domain: &'static str,
    key: &'static str,
    current_host: bool,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed: Option<String>,
    description: &'static str,
    /// `null` when unset.
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn list(prefs: &dyn Prefs, category: Option<&str>, json: bool) -> Result<()> {
    let settings: Vec<_> =
        CATALOG.iter().filter(|s| category.is_none_or(|c| s.category.eq_ignore_ascii_case(c))).collect();
    if settings.is_empty() {
        bail!("no category {:?}; categories: {}", category.unwrap_or(""), catalog::categories().join(", "));
    }
    if json {
        let rows: Vec<_> = settings
            .iter()
            .map(|s| {
                let (value, error) = match prefs.read(s) {
                    Ok(v) => (v, None),
                    Err(e) => (None, Some(format!("{e:#}"))),
                };
                ListRow {
                    id: s.id,
                    category: s.category,
                    label: s.label,
                    domain: s.domain,
                    key: s.key,
                    current_host: s.host,
                    kind: format!("{:?}", s.kind).to_lowercase(),
                    allowed: s.allowed_text(),
                    description: s.description,
                    value,
                    error,
                }
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    let mut current = "";
    for s in settings {
        if s.category != current {
            println!("{}{}", if current.is_empty() { "" } else { "\n" }, s.category);
            current = s.category;
        }
        let value = match prefs.read(s) {
            Ok(v) => value::show(&v),
            Err(e) => format!("error: {e:#}"),
        };
        println!("  {:<34} {:<40} {value}", s.id, s.label);
    }
    Ok(())
}

fn export(prefs: &dyn Prefs, output: PathBuf, ids: &[String], changed: bool) -> Result<()> {
    let settings: Vec<_> = if ids.is_empty() {
        CATALOG.iter().collect()
    } else {
        let mut out = Vec::new();
        for id in ids {
            let id = id.trim();
            let matched: Vec<_> = match catalog::find(id) {
                Some(s) => vec![s],
                None => CATALOG.iter().filter(|s| s.category.eq_ignore_ascii_case(id)).collect(),
            };
            if matched.is_empty() {
                bail!("unknown setting or category {id:?} (see `macpack list`)");
            }
            for s in matched {
                if !out.iter().any(|o: &&catalog::Setting| o.id == s.id) {
                    out.push(s);
                }
            }
        }
        out
    };
    let (mut profile, warnings) = Profile::capture(prefs, &settings);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    if changed {
        profile.settings.retain(|e| e.value.is_some());
    }
    profile.save(&output)?;
    println!("Saved {} setting(s) to {}", profile.settings.len(), output.display());
    Ok(())
}

/// Prints a plan's warnings and changes against `name`.
fn print_plan(plan: &Plan, name: &str) {
    for w in &plan.warnings {
        eprintln!("warning: {w}");
    }
    if plan.changes.is_empty() {
        println!("This Mac already matches {name} ({} settings).", plan.unchanged);
        return;
    }
    println!("{:<34} {:<24} PROFILE", "SETTING", "THIS MAC");
    for c in &plan.changes {
        println!("{:<34} {:<24} {}", c.setting.id, value::show(&c.current), value::show(&c.desired));
    }
    println!("{} change(s), {} already match.", plan.changes.len(), plan.unchanged);
}

fn apply_cli(prefs: &dyn Prefs, path: &Path, dry_run: bool, restart: bool) -> Result<()> {
    let profile = Profile::load(path)?;
    apply_profile(prefs, &profile, &path.display().to_string(), dry_run, restart)
}

fn apply_profile(prefs: &dyn Prefs, profile: &Profile, name: &str, dry_run: bool, restart: bool) -> Result<()> {
    let plan = Plan::new(profile, prefs);
    print_plan(&plan, name);
    if dry_run || plan.changes.is_empty() {
        return Ok(());
    }

    let out = apply::apply(&plan, prefs, Some(&backup::dir()), restart)?;
    println!("\nApplied {} setting(s).", out.applied.len());
    if let Some(b) = &out.backup {
        println!("Backup of previous values: {}  (undo: macpack restore latest)", b.display());
    }
    if !out.restarted.is_empty() {
        println!("Restarted: {}", out.restarted.join(", "));
    }
    if apply::needs_logout(&out) {
        println!("Some settings take effect after you log out or relaunch apps.");
    }
    for e in &out.errors {
        eprintln!("failed: {e}");
    }
    if !out.errors.is_empty() {
        bail!("{} setting(s) could not be applied", out.errors.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_is_well_formed() {
        Cli::command().debug_assert();
        assert!(Cli::try_parse_from(["macpack", "set", "appearance.accent-color", "-1"]).is_ok());
        assert!(Cli::try_parse_from(["macpack", "set", "dock.orientation"]).is_err());
        assert!(Cli::try_parse_from(["macpack", "set", "dock.orientation", "left", "--unset"]).is_err());
    }
}
