use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::app::App;

use super::theme;

pub fn render(frame: &mut Frame, app: &App) {
    if !app.overlays.command_palette.active {
        return;
    }

    let area = centered_rect(64, 48, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Commands")
        .borders(Borders::ALL)
        .style(theme::panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(format!("Query: {}", app.overlays.command_palette.query))
            .style(theme::panel_style()),
        layout[0],
    );

    let (items, selected_display_index) = build_palette_items(app);
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .style(theme::panel_style());
    let mut state = ListState::default();
    if !app.overlays.command_palette.entries.is_empty() {
        state.select(selected_display_index);
    }
    frame.render_stateful_widget(list, layout[1], &mut state);

    frame.render_widget(
        Paragraph::new("Type to filter  Up/Down move  Enter choose  Esc cancel")
            .style(theme::panel_style()),
        layout[2],
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

fn build_palette_items(app: &App) -> (Vec<ListItem<'static>>, Option<usize>) {
    if app.overlays.command_palette.entries.is_empty() {
        return (vec![ListItem::new("No actions")], None);
    }

    let mut items = Vec::new();
    let mut selected_display_index = None;
    let mut previous_section = None;
    let show_sections = app.overlays.command_palette.query.is_empty();

    for (entry_index, entry) in app.overlays.command_palette.entries.iter().enumerate() {
        if show_sections {
            let section = crate::actions::catalog::command_palette_section_label(&entry.action);
            if previous_section != Some(section) {
                items.push(ListItem::new(Line::from(Span::styled(
                    section.to_string(),
                    theme::command_palette_section_style(),
                ))));
                previous_section = Some(section);
            }
        }

        let mut spans = vec![Span::styled(entry.label.clone(), theme::panel_style())];
        if let Some(subtitle) = entry.subtitle.as_deref() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                subtitle.to_string(),
                theme::command_palette_subtitle_style(),
            ));
        }
        if entry_index == app.overlays.command_palette.selected_index {
            selected_display_index = Some(items.len());
        }
        items.push(ListItem::new(Line::from(spans)));
    }

    (items, selected_display_index)
}
