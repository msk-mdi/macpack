use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};

use super::{accent, hints};
use crate::apply::Plan;
use crate::value;

pub enum Outcome {
    Stay,
    Back,
    Apply,
}

/// The apply screen: current vs. profile values, with per-row opt-out.
pub struct DiffState {
    pub title: String,
    pub plan: Plan,
    table: TableState,
    confirming: bool,
}

impl DiffState {
    pub fn new(title: String, plan: Plan) -> Self {
        DiffState { title, plan, table: TableState::default().with_selected(Some(0)), confirming: false }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        if self.confirming {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => return Outcome::Apply,
                _ => self.confirming = false,
            }
            return Outcome::Stay;
        }
        let len = self.plan.changes.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Outcome::Back,
            KeyCode::Up | KeyCode::Char('k') => self.table.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.table.select_next(),
            KeyCode::Char(' ') => {
                if let Some(c) = self.table.selected().and_then(|i| self.plan.changes.get_mut(i)) {
                    c.selected = !c.selected;
                }
                self.table.select_next();
            }
            KeyCode::Char('a') => {
                let all = self.plan.changes.iter().all(|c| c.selected);
                self.plan.changes.iter_mut().for_each(|c| c.selected = !all);
            }
            KeyCode::Enter if self.plan.selected().next().is_some() => self.confirming = true,
            _ => {}
        }
        if let Some(i) = self.table.selected() {
            self.table.select(Some(i.min(len.saturating_sub(1))));
        }
        Outcome::Stay
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let warn_height = self.plan.warnings.len().min(4) as u16;
        let [main, detail, warnings, footer] = Layout::vertical([
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(warn_height),
            Constraint::Length(1),
        ])
        .areas(area);

        let header = Row::new(["", "Category", "Setting", "This Mac", "Profile"])
            .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::DarkGray));
        let rows: Vec<Row> = self
            .plan
            .changes
            .iter()
            .map(|c| {
                let dim = if c.selected { Style::default() } else { Style::default().fg(Color::DarkGray) };
                Row::new([
                    Cell::from(if c.selected { "[x]" } else { "[ ]" }).style(accent()),
                    Cell::from(c.setting.category),
                    Cell::from(c.setting.label),
                    Cell::from(value::show(&c.current)).style(if c.selected { Style::default().fg(Color::Red) } else { dim }),
                    Cell::from(value::show(&c.desired)).style(if c.selected { Style::default().fg(Color::Green) } else { dim }),
                ])
                .style(dim)
            })
            .collect();
        let widths = [
            Constraint::Length(3),
            Constraint::Length(20),
            Constraint::Fill(2),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ];
        let n = self.plan.selected().count();
        let summary = format!(" {n} of {} changes selected · {} already match ", self.plan.changes.len(), self.plan.unchanged);
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::bordered().title(format!(" {} ", self.title)).title_bottom(Line::from(summary).right_aligned()).border_style(accent()))
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(table, main, &mut self.table);

        if let Some(c) = self.table.selected().and_then(|i| self.plan.changes.get(i)) {
            let s = c.setting;
            let text = vec![
                Line::from(vec![s.label.bold(), format!("  {}  {} {}", s.id, s.domain, s.key).dark_gray()]),
                Line::from(match s.allowed_text() {
                    Some(allowed) => format!("{}  Allowed: {allowed}.", s.description),
                    None => s.description.to_string(),
                }),
            ];
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), detail);
        }

        let lines: Vec<Line> = self.plan.warnings.iter().map(|w| Line::from(format!("! {w}").yellow())).collect();
        f.render_widget(Paragraph::new(lines), warnings);
        f.render_widget(
            hints(&[("↑↓", "move"), ("space", "toggle"), ("a", "all/none"), ("enter", "apply"), ("esc", "back")]),
            footer,
        );

        if self.confirming {
            self.draw_confirm(f, area, n);
        }
    }

    fn draw_confirm(&self, f: &mut Frame, area: Rect, n: usize) {
        let [area] = Layout::horizontal([Constraint::Length(56)]).flex(Flex::Center).areas(area);
        let [area] = Layout::vertical([Constraint::Length(7)]).flex(Flex::Center).areas(area);
        let text = vec![
            Line::from(format!("Apply {n} change{}?", if n == 1 { "" } else { "s" }).bold()),
            Line::from(""),
            Line::from("Current values are backed up first, so you can"),
            Line::from("undo with `macpack restore latest`."),
            Line::from(vec!["y/enter".bold(), " apply   ".into(), "any other key".bold(), " cancel".into()]),
        ];
        f.render_widget(Clear, area);
        f.render_widget(Paragraph::new(text).block(Block::bordered().border_style(accent())), area);
    }
}
