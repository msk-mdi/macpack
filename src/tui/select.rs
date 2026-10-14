use std::collections::{BTreeSet, HashMap};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use super::{accent, hints};
use crate::catalog::{self, CATALOG, Setting};
use crate::defaults::Prefs;
use crate::value::{self, Value};

#[derive(PartialEq)]
enum Focus {
    Categories,
    Settings,
}

pub enum Outcome {
    Stay,
    Back,
    Done,
}

/// The export screen: pick settings by category.
pub struct SelectState {
    categories: Vec<&'static str>,
    values: HashMap<&'static str, Result<Option<Value>, String>>,
    selected: BTreeSet<&'static str>,
    cat: ListState,
    item: ListState,
    focus: Focus,
    filter: String,
    filtering: bool,
}

impl SelectState {
    pub fn new(prefs: &dyn Prefs) -> Self {
        let values = CATALOG
            .iter()
            .map(|s| (s.id, prefs.read(s).map_err(|e| format!("{e:#}"))))
            .collect();
        SelectState {
            categories: catalog::categories(),
            values,
            selected: BTreeSet::new(),
            cat: ListState::default().with_selected(Some(0)),
            item: ListState::default().with_selected(Some(0)),
            focus: Focus::Categories,
            filter: String::new(),
            filtering: false,
        }
    }

    /// Selected settings with the values shown on screen, in catalog order.
    pub fn chosen(&self) -> Vec<(&'static Setting, Result<Option<Value>, String>)> {
        CATALOG
            .iter()
            .filter(|s| self.selected.contains(s.id))
            .map(|s| (s, self.values[s.id].clone()))
            .collect()
    }

    fn visible(&self) -> Vec<&'static Setting> {
        if !self.filter.is_empty() {
            let needle = self.filter.to_lowercase();
            return CATALOG
                .iter()
                .filter(|s| {
                    [s.label, s.id, s.key, s.category].iter().any(|t| t.to_lowercase().contains(&needle))
                })
                .collect();
        }
        let cat = self.categories[self.cat.selected().unwrap_or(0)];
        CATALOG.iter().filter(|s| s.category == cat).collect()
    }

    fn current(&self) -> Option<&'static Setting> {
        self.visible().get(self.item.selected()?).copied()
    }

    fn toggle(&mut self, id: &'static str) {
        if !self.selected.remove(id) {
            self.selected.insert(id);
        }
    }

    /// Selects every visible setting, or clears them if all are selected.
    fn toggle_all(&mut self) {
        let ids: Vec<_> = self.visible().iter().map(|s| s.id).collect();
        if ids.iter().all(|id| self.selected.contains(id)) {
            ids.iter().for_each(|id| {
                self.selected.remove(id);
            });
        } else {
            self.selected.extend(ids);
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                }
                KeyCode::Enter | KeyCode::Down => {
                    self.filtering = false;
                    self.focus = Focus::Settings;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.item.select(Some(0));
            return Outcome::Stay;
        }
        let len = self.visible().len();
        match key.code {
            KeyCode::Char('q') => return Outcome::Back,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.item.select(Some(0));
            }
            KeyCode::Esc => return Outcome::Back,
            KeyCode::Enter => return Outcome::Done,
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter.clear();
            }
            KeyCode::Char('a') => self.toggle_all(),
            KeyCode::Char('n') => self.selected.clear(),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') if self.focus == Focus::Categories => {
                self.focus = Focus::Settings
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') | KeyCode::Tab if self.focus == Focus::Settings => {
                self.focus = Focus::Categories
            }
            KeyCode::Char(' ') => match self.focus {
                Focus::Settings => {
                    if let Some(s) = self.current() {
                        self.toggle(s.id);
                    }
                    self.item.select(Some((self.item.selected().unwrap_or(0) + 1).min(len.saturating_sub(1))));
                }
                Focus::Categories => self.toggle_all(),
            },
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            _ => {}
        }
        Outcome::Stay
    }

    fn step(&mut self, delta: isize) {
        let on_categories = self.focus == Focus::Categories && self.filter.is_empty();
        let (state, len) = if on_categories {
            (&mut self.cat, self.categories.len())
        } else {
            let len = self.visible().len();
            (&mut self.item, len)
        };
        let i = state.selected().unwrap_or(0) as isize + delta;
        state.select(Some(i.clamp(0, len.saturating_sub(1) as isize) as usize));
        if on_categories {
            self.item.select(Some(0));
        }
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let [main, detail, footer] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(4), Constraint::Length(1)]).areas(area);
        let [left, right] = Layout::horizontal([Constraint::Length(24), Constraint::Min(30)]).areas(main);

        let focus_style = |on: bool| if on { accent() } else { Style::default().fg(Color::DarkGray) };

        // Categories with selection counts.
        let items: Vec<ListItem> = self
            .categories
            .iter()
            .map(|c| {
                let n = CATALOG.iter().filter(|s| s.category == *c && self.selected.contains(s.id)).count();
                let count = if n > 0 { format!(" {n}") } else { String::new() };
                ListItem::new(Line::from(vec![Span::raw(*c), Span::styled(count, accent())]))
            })
            .collect();
        let cats = List::new(items)
            .block(Block::bordered().title(" Categories ").border_style(focus_style(self.focus == Focus::Categories)))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        if self.filter.is_empty() {
            f.render_stateful_widget(cats, left, &mut self.cat);
        } else {
            f.render_widget(cats, left);
        }

        // Settings with checkboxes and current values.
        let visible = self.visible();
        let label_width = right.width.saturating_sub(4).saturating_mul(3) / 5;
        let items: Vec<ListItem> = visible
            .iter()
            .map(|s| {
                let check = if self.selected.contains(s.id) { "[x] " } else { "[ ] " };
                let value = match &self.values[s.id] {
                    Ok(v) => Span::styled(value::show(v), Style::default().fg(if v.is_some() { Color::Cyan } else { Color::DarkGray })),
                    Err(_) => Span::styled("unreadable", Style::default().fg(Color::Red)),
                };
                let label = if self.filter.is_empty() { s.label.to_string() } else { format!("{} › {}", s.category, s.label) };
                Line::from(vec![
                    Span::styled(check, accent()),
                    Span::raw(format!("{label:<w$} ", w = label_width as usize)),
                    value,
                ])
                .into()
            })
            .collect();
        let title = if self.filtering || !self.filter.is_empty() {
            format!(" Filter: {}{} ", self.filter, if self.filtering { "▏" } else { "" })
        } else {
            format!(" {} ", self.categories[self.cat.selected().unwrap_or(0)])
        };
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(title)
                    .title_bottom(Line::from(format!(" {} selected ", self.selected.len())).right_aligned())
                    .border_style(focus_style(self.focus == Focus::Settings || self.filtering)),
            )
            .highlight_style(if self.focus == Focus::Settings {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            });
        f.render_stateful_widget(list, right, &mut self.item);

        // Details of the highlighted setting.
        let text = match self.current() {
            Some(s) => {
                let host = if s.host { " -currentHost" } else { "" };
                let mut lines = vec![
                    Line::from(vec![s.label.bold(), format!("  {}", s.id).dark_gray()]),
                    Line::from(s.description),
                    Line::from(format!("defaults{host} {} {}", s.domain, s.key).dark_gray()),
                ];
                if let Err(e) = &self.values[s.id] {
                    lines.push(Line::from(e.clone().red()));
                }
                lines
            }
            None => vec![Line::from("No matching settings".dark_gray())],
        };
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), detail);

        let keys: &[(&str, &str)] = if self.filtering {
            &[("type", "filter"), ("enter", "done"), ("esc", "clear")]
        } else {
            &[("↑↓", "move"), ("←→", "pane"), ("space", "toggle"), ("a", "all"), ("n", "none"), ("/", "filter"), ("enter", "export"), ("esc", "back")]
        };
        f.render_widget(hints(keys), footer);
    }
}
