use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::app::App;

use super::theme;

pub fn render(frame: &mut Frame, app: &App) {
    if !app.overlays.context_menu.active {
        return;
    }

    let area = centered_rect(48, 40, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Context Menu")
        .borders(Borders::ALL)
        .style(theme::panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(2)])
        .split(inner);

    let items = app
        .overlays
        .context_menu
        .entries
        .iter()
        .map(|entry| {
            let subtitle = entry.subtitle.as_deref().unwrap_or_default();
            ListItem::new(format!("{}  {}", entry.label, subtitle))
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .style(theme::panel_style());
    let mut state = ListState::default();
    if !app.overlays.context_menu.entries.is_empty() {
        state.select(Some(app.overlays.context_menu.selected_index));
    }
    frame.render_stateful_widget(list, layout[0], &mut state);

    frame.render_widget(
        Paragraph::new("Up/Down move  Enter choose  Esc cancel").style(theme::panel_style()),
        layout[1],
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical_margin = (100_u16.saturating_sub(percent_y)) / 2;
    let horizontal_margin = (100_u16.saturating_sub(percent_x)) / 2;
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(vertical_margin),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(vertical_margin),
        ])
        .split(area);
    let vertical = popup_layout[1];

    let horizontal_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(horizontal_margin),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(horizontal_margin),
        ])
        .split(vertical);
    horizontal_layout[1]
}
