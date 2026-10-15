mod diff;
mod select;

use std::path::{Path, PathBuf};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::apply::{self, Plan};
use crate::backup;
use crate::defaults::Prefs;
use crate::profile::{self, Profile};
use diff::DiffState;
use select::SelectState;

/// Where the TUI opens.
pub enum Start {
    Home,
    Export(Option<PathBuf>),
    Apply(PathBuf),
}

pub fn run(prefs: &dyn Prefs, start: Start) -> Result<()> {
    let mut app = App { prefs, screen: Screen::Home(home_state()), quit: false, export_path: None };
    app.screen = match start {
        Start::Home => Screen::Home(home_state()),
        Start::Export(path) => {
            app.export_path = path;
            Screen::Select(Box::new(SelectState::new(prefs)))
        }
        Start::Apply(path) => app.open_profile(&path, false),
    };
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

pub(crate) fn accent() -> Style {
    Style::default().fg(Color::Magenta)
}

/// A footer line of `key description` pairs.
pub(crate) fn hints(keys: &[(&str, &str)]) -> Paragraph<'static> {
    let mut spans = Vec::new();
    for (key, what) in keys {
        spans.push(Span::styled(format!(" {key} "), accent().add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!("{what}  "), Style::default().fg(Color::DarkGray)));
    }
    Paragraph::new(Line::from(spans))
}

const HOME_ITEMS: [(&str, &str); 4] = [
    ("Export settings", "Choose settings on this Mac and save them to a profile"),
    ("Apply a profile", "Review a profile's changes against this Mac and apply them"),
    ("Restore a backup", "Undo a previous apply"),
    ("Quit", ""),
];

fn home_state() -> ListState {
    ListState::default().with_selected(Some(0))
}

enum InputPurpose {
    /// Save the selection from this (boxed) select screen.
    Export(Box<SelectState>),
    Apply,
}

struct InputState {
    prompt: &'static str,
    value: String,
    purpose: InputPurpose,
}

enum Screen {
    Home(ListState),
    Select(Box<SelectState>),
    Input(InputState),
    Diff(Box<DiffState>),
    Backups(Vec<PathBuf>, ListState),
    Message(String, Vec<Line<'static>>),
}

struct App<'a> {
    prefs: &'a dyn Prefs,
    screen: Screen,
    quit: bool,
    /// Export path passed on the command line.
    export_path: Option<PathBuf>,
}

impl App<'_> {
    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                // Redraw with a status line while slow work (reading every
                // setting, applying) runs.
                if self.is_slow(key) {
                    terminal.draw(|f| {
                        self.draw(f);
                        let area = f.area();
                        let line = Rect { y: area.height.saturating_sub(1), height: 1, ..area };
                        f.render_widget(Paragraph::new(" Working…".bold().style(accent())), line);
                    })?;
                }
                self.on_key(key);
            }
        }
        Ok(())
    }

    fn is_slow(&self, key: KeyEvent) -> bool {
        match &self.screen {
            Screen::Home(state) => key.code == KeyCode::Enter && state.selected() == Some(0),
            Screen::Diff(_) | Screen::Input(_) | Screen::Backups(..) => matches!(key.code, KeyCode::Enter | KeyCode::Char('y')),
            _ => false,
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let screen = std::mem::replace(&mut self.screen, Screen::Home(home_state()));
        self.screen = match screen {
            Screen::Home(mut state) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    state.select_previous();
                    Screen::Home(state)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    state.select(Some((state.selected().unwrap_or(0) + 1).min(HOME_ITEMS.len() - 1)));
                    Screen::Home(state)
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    self.quit = true;
                    Screen::Home(state)
                }
                KeyCode::Enter => match state.selected() {
                    Some(0) => Screen::Select(Box::new(SelectState::new(self.prefs))),
                    Some(1) => Screen::Input(InputState {
                        prompt: "Profile to apply",
                        value: profile::tilde(&profile::default_path()),
                        purpose: InputPurpose::Apply,
                    }),
                    Some(2) => match backup::list(&backup::dir()) {
                        Ok(list) if list.is_empty() => message("Restore", vec!["No backups yet. One is made every time you apply a profile.".into()]),
                        Ok(list) => Screen::Backups(list, home_state()),
                        Err(e) => error("Restore", e),
                    },
                    _ => {
                        self.quit = true;
                        Screen::Home(state)
                    }
                },
                _ => Screen::Home(state),
            },

            Screen::Select(mut state) => match state.on_key(key) {
                select::Outcome::Stay => Screen::Select(state),
                select::Outcome::Back => Screen::Home(home_state()),
                select::Outcome::Done if state.chosen().is_empty() => {
                    // Nothing selected: stay put.
                    Screen::Select(state)
                }
                select::Outcome::Done => {
                    let path = self.export_path.clone().unwrap_or_else(profile::default_path);
                    Screen::Input(InputState {
                        prompt: "Save profile to",
                        value: profile::tilde(&path),
                        purpose: InputPurpose::Export(state),
                    })
                }
            },

            Screen::Input(mut input) => match key.code {
                KeyCode::Esc => match input.purpose {
                    InputPurpose::Export(state) => Screen::Select(state),
                    InputPurpose::Apply => Screen::Home(home_state()),
                },
                KeyCode::Backspace => {
                    input.value.pop();
                    Screen::Input(input)
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.value.clear();
                    Screen::Input(input)
                }
                KeyCode::Char(c) => {
                    input.value.push(c);
                    Screen::Input(input)
                }
                KeyCode::Enter => {
                    let path = profile::expand(input.value.trim());
                    match input.purpose {
                        InputPurpose::Export(state) => export(&state, &path),
                        InputPurpose::Apply => self.open_profile(&path, false),
                    }
                }
                _ => Screen::Input(input),
            },

            Screen::Diff(mut state) => match state.on_key(key) {
                diff::Outcome::Stay => Screen::Diff(state),
                diff::Outcome::Back => Screen::Home(home_state()),
                diff::Outcome::Apply => self.apply(&state.plan),
            },

            Screen::Backups(list, mut state) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    state.select_previous();
                    Screen::Backups(list, state)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    state.select(Some((state.selected().unwrap_or(0) + 1).min(list.len() - 1)));
                    Screen::Backups(list, state)
                }
                KeyCode::Enter => self.open_profile(&list[state.selected().unwrap_or(0)], true),
                KeyCode::Esc | KeyCode::Char('q') => Screen::Home(home_state()),
                _ => Screen::Backups(list, state),
            },

            Screen::Message(..) => Screen::Home(home_state()),
        };
    }

    /// Loads a profile and shows what applying it would change.
    fn open_profile(&self, path: &Path, restore: bool) -> Screen {
        let verb = if restore { "Restore" } else { "Apply" };
        let profile = match Profile::load(path) {
            Ok(p) => p,
            Err(e) => return error(verb, e),
        };
        let plan = Plan::new(&profile, self.prefs);
        if plan.changes.is_empty() {
            let mut lines = vec![Line::from(format!(
                "This Mac already matches {} ({} settings).",
                profile::tilde(path),
                plan.unchanged
            ))];
            lines.extend(plan.warnings.iter().map(|w| Line::from(format!("! {w}").yellow())));
            return message(verb, lines);
        }
        Screen::Diff(Box::new(DiffState::new(format!("{verb} {}", profile::tilde(path)), plan)))
    }

    fn apply(&self, plan: &Plan) -> Screen {
        let out = match apply::apply(plan, self.prefs, Some(&backup::dir()), true) {
            Ok(out) => out,
            Err(e) => return error("Apply", e),
        };
        let mut lines = vec![Line::from(format!("Applied {} setting(s).", out.applied.len()).green().bold())];
        if let Some(b) = &out.backup {
            lines.push(Line::from(format!("Backup of previous values: {}", profile::tilde(b))));
            lines.push(Line::from("Undo with: macpack restore latest".dark_gray()));
        }
        if !out.restarted.is_empty() {
            lines.push(Line::from(format!("Restarted: {}", out.restarted.join(", "))));
        }
        if apply::needs_logout(&out) {
            lines.push(Line::from("Some settings take effect after you log out or relaunch apps.".yellow()));
        }
        for e in &out.errors {
            lines.push(Line::from(format!("✗ {e}").red()));
        }
        message("Done", lines)
    }

    fn draw(&mut self, f: &mut Frame) {
        let [header, body] = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(f.area());
        f.render_widget(
            Paragraph::new(Line::from(vec![
                " macpack ".bold().style(accent().add_modifier(Modifier::REVERSED)),
                "  portable macOS settings".dark_gray(),
            ])),
            header,
        );
        match &mut self.screen {
            Screen::Home(state) => {
                let items: Vec<ListItem> = HOME_ITEMS
                    .iter()
                    .map(|(title, desc)| {
                        ListItem::new(vec![Line::from(title.bold()), Line::from(desc.dark_gray()), Line::from("")])
                    })
                    .collect();
                let [list, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(body);
                let list_widget = List::new(items).highlight_symbol("▶ ").highlight_style(accent());
                f.render_stateful_widget(list_widget, list.inner(ratatui::layout::Margin::new(1, 0)), state);
                f.render_widget(hints(&[("↑↓", "move"), ("enter", "open"), ("q", "quit")]), footer);
            }
            Screen::Select(state) => state.draw(f, body),
            Screen::Diff(state) => state.draw(f, body),
            Screen::Input(input) => {
                let [field, help, _, footer] = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Length(2),
                    Constraint::Min(0),
                    Constraint::Length(1),
                ])
                .areas(body);
                f.render_widget(
                    Paragraph::new(format!("{}▏", input.value))
                        .block(Block::bordered().title(format!(" {} ", input.prompt)).border_style(accent())),
                    field,
                );
                let help_text = match &input.purpose {
                    InputPurpose::Export(state) => format!(" {} setting(s) will be saved. `~` means your home folder.", state.chosen().len()),
                    InputPurpose::Apply => " Path to a profile created with macpack export.".to_string(),
                };
                f.render_widget(Paragraph::new(help_text.dark_gray()), help);
                f.render_widget(hints(&[("enter", "confirm"), ("ctrl-u", "clear"), ("esc", "back")]), footer);
            }
            Screen::Backups(list, state) => {
                let items: Vec<ListItem> = list
                    .iter()
                    .map(|p| {
                        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        let count = Profile::load(p).map(|pr| format!("{} settings", pr.settings.len())).unwrap_or_else(|_| "unreadable".into());
                        ListItem::new(Line::from(vec![Span::raw(format!("{name:<28}")), count.dark_gray()]))
                    })
                    .collect();
                let [list_area, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(body);
                let widget = List::new(items)
                    .block(Block::bordered().title(" Backups (newest first) ").border_style(accent()))
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
                f.render_stateful_widget(widget, list_area, state);
                f.render_widget(hints(&[("↑↓", "move"), ("enter", "review & restore"), ("esc", "back")]), footer);
            }
            Screen::Message(title, lines) => {
                let [text, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(body);
                f.render_widget(
                    Paragraph::new(lines.clone())
                        .wrap(Wrap { trim: false })
                        .block(Block::bordered().title(format!(" {title} ")).border_style(accent())),
                    text,
                );
                f.render_widget(hints(&[("any key", "continue")]), footer);
            }
        }
    }
}

fn export(state: &SelectState, path: &Path) -> Screen {
    let mut skipped = Vec::new();
    let mut entries = Vec::new();
    for (setting, value) in state.chosen() {
        match value {
            Ok(v) => entries.push((setting, v)),
            Err(e) => skipped.push(format!("skipped {}: {e}", setting.id)),
        }
    }
    let count = entries.len();
    if let Err(e) = Profile::new(entries).save(path) {
        return error("Export", e);
    }
    let mut lines = vec![
        Line::from(format!("Saved {count} setting(s) to {}", profile::tilde(path)).green().bold()),
        Line::from(""),
        Line::from("Copy it to another Mac and run:"),
        Line::from(format!("  macpack apply {}", path.file_name().unwrap_or_default().to_string_lossy()).bold()),
    ];
    lines.extend(skipped.into_iter().map(|w| Line::from(format!("! {w}").yellow())));
    message("Export", lines)
}

fn message(title: &str, lines: Vec<Line<'static>>) -> Screen {
    Screen::Message(title.to_string(), lines)
}

fn error(title: &str, e: anyhow::Error) -> Screen {
    message(title, vec![Line::from(format!("{e:#}").red())])
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::defaults::mem::MemPrefs;
    use crate::value::Value;

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn render(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn walks_export_flow() {
        let prefs = MemPrefs::with(&[("dock.autohide", Value::Bool(true))]);
        let mut app = App { prefs: &prefs, screen: Screen::Home(home_state()), quit: false, export_path: None };
        assert!(render(&mut app).contains("Export settings"));

        press(&mut app, KeyCode::Enter);
        assert!(render(&mut app).contains("Automatically hide the Dock"));
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('/'));
        for c in "finder".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let screen = render(&mut app);
        assert!(screen.contains("Filter: finder"), "{screen}");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char(' '));
        assert!(render(&mut app).contains("2 selected"));
        // Changing a value selects the setting and marks it as edited.
        press(&mut app, KeyCode::Char('c'));
        let screen = render(&mut app);
        assert!(screen.contains("true (edited)"), "{screen}");
        assert!(screen.contains("3 selected"), "{screen}");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.toml");
        app.export_path = Some(path.clone());
        press(&mut app, KeyCode::Enter);
        assert!(render(&mut app).contains("3 setting(s) will be saved"));
        press(&mut app, KeyCode::Enter);
        assert!(render(&mut app).contains("Saved 3 setting(s)"));
        let saved = Profile::load(&path).unwrap();
        let edited = saved.settings.iter().find(|e| e.id == "finder.show-extensions").unwrap();
        assert_eq!(edited.value, Some(Value::Bool(true)));

        // The saved profile opens as a diff against a Mac that differs.
        let other = MemPrefs::default();
        let app2 = App { prefs: &other, screen: Screen::Home(home_state()), quit: false, export_path: None };
        let mut app2 = App { screen: app2.open_profile(&path, false), ..app2 };
        let screen = render(&mut app2);
        assert!(screen.contains("2 of 2 changes selected"), "{screen}");
        press(&mut app2, KeyCode::Char(' '));
        assert!(render(&mut app2).contains("1 of 2 changes selected"));
        press(&mut app2, KeyCode::Esc);
        assert!(render(&mut app2).contains("Apply a profile"));
    }
}
