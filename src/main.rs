mod apply;
mod backup;
mod catalog;
mod defaults;
mod profile;
mod tui;
mod value;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use crate::apply::Plan;
use crate::catalog::CATALOG;
use crate::defaults::{DefaultsCli, Prefs};
use crate::profile::Profile;

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
    },
    /// Save settings to a profile. Opens the picker unless --all or --ids is given.
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
        /// Don't restart Dock, Finder or SystemUIServer afterwards.
        #[arg(long)]
        no_restart: bool,
    },
    /// List backups made before each apply.
    Backups,
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
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let prefs = DefaultsCli;
    match cli.command {
        None => interactive(&prefs, tui::Start::Home),
        Some(Command::List { category }) => list(&prefs, category.as_deref()),
        Some(Command::Export { output, all, ids }) => {
            if !all && ids.is_empty() {
                return interactive(&prefs, tui::Start::Export(output));
            }
            export(&prefs, output.unwrap_or_else(profile::default_path), all, &ids)
        }
        Some(Command::Apply { profile, dry_run, yes, no_restart }) => {
            if !dry_run && !yes {
                Profile::load(&profile)?; // fail fast before opening the TUI
                return interactive(&prefs, tui::Start::Apply(profile));
            }
            apply_cli(&prefs, &profile, dry_run, !no_restart)
        }
        Some(Command::Backups) => {
            let list = backup::list(&backup::dir())?;
            if list.is_empty() {
                println!("No backups yet in {}", profile::tilde(&backup::dir()));
            }
            for p in list {
                let n = Profile::load(&p).map(|pr| pr.settings.len().to_string()).unwrap_or_else(|_| "?".into());
                println!("{:<28} {n:>3} settings", p.file_stem().unwrap_or_default().to_string_lossy());
            }
            Ok(())
        }
        Some(Command::Restore { backup: name, dry_run, yes, no_restart }) => {
            let path = backup::find(&backup::dir(), &name)?;
            if !dry_run && !yes {
                return interactive(&prefs, tui::Start::Apply(path));
            }
            apply_cli(&prefs, &path, dry_run, !no_restart)
        }
    }
}

fn interactive(prefs: &dyn Prefs, start: tui::Start) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        bail!("the interactive interface needs a terminal; see `macpack --help` for non-interactive commands");
    }
    tui::run(prefs, start)
}

fn list(prefs: &dyn Prefs, category: Option<&str>) -> Result<()> {
    let mut current = "";
    let mut shown = 0;
    for s in CATALOG {
        if category.is_some_and(|c| !s.category.eq_ignore_ascii_case(c)) {
            continue;
        }
        if s.category != current {
            current = s.category;
            println!("{}{current}", if shown > 0 { "\n" } else { "" });
        }
        let value = match prefs.read(s) {
            Ok(v) => value::show(&v),
            Err(e) => format!("error: {e:#}"),
        };
        println!("  {:<34} {:<40} {value}", s.id, s.label);
        shown += 1;
    }
    if shown == 0 {
        bail!("no category {:?}; categories: {}", category.unwrap_or(""), catalog::categories().join(", "));
    }
    Ok(())
}

fn export(prefs: &dyn Prefs, output: PathBuf, all: bool, ids: &[String]) -> Result<()> {
    let settings: Vec<_> = if all {
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
    let (profile, warnings) = Profile::capture(prefs, &settings);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    profile.save(&output)?;
    println!("Saved {} setting(s) to {}", profile.settings.len(), output.display());
    Ok(())
}

fn apply_cli(prefs: &dyn Prefs, path: &Path, dry_run: bool, restart: bool) -> Result<()> {
    let profile = Profile::load(path)?;
    let plan = Plan::new(&profile, prefs);
    for w in &plan.warnings {
        eprintln!("warning: {w}");
    }
    if plan.changes.is_empty() {
        println!("This Mac already matches {} ({} settings).", path.display(), plan.unchanged);
        return Ok(());
    }
    println!("{:<34} {:<24} PROFILE", "SETTING", "THIS MAC");
    for c in &plan.changes {
        println!("{:<34} {:<24} {}", c.setting.id, value::show(&c.current), value::show(&c.desired));
    }
    println!("{} change(s), {} already match.", plan.changes.len(), plan.unchanged);
    if dry_run {
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
