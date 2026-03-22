use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::app::App;
use crate::bridge::protocol::{SessionSummary, TargetRole};

use super::theme;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(picker) = app.target_picker_state() else {
        return;
    };

    let area = centered_rect(70, 62, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(title_for_role(picker.role))
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

    let header = Paragraph::new(header_text(app, picker.role)).style(theme::panel_style());
    frame.render_widget(header, layout[0]);

    let items = session_items(app);
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .style(theme::panel_style());
    let mut state = ListState::default();
    if let Some(selected_index) = app.target_picker_selected_index() {
        state.select(Some(selected_index));
    }
    frame.render_stateful_widget(list, layout[1], &mut state);

    let footer =
        Paragraph::new("Up/Down select  Enter choose  Esc cancel").style(theme::panel_style());
    frame.render_widget(footer, layout[2]);
}

fn header_text(app: &App, role: TargetRole) -> String {
    let selected = app.target_picker_current_label(role);
    format!(
        "Select {} target. Current: {}",
        title_for_role(role).trim_start_matches("Select "),
        selected
    )
}

fn session_items(app: &App) -> Vec<ListItem<'static>> {
    let Some(picker) = app.target_picker_state() else {
        return vec![ListItem::new("No sessions available")];
    };
    let has_current_pane = picker.role == TargetRole::Editor;
    if app.bridge.session_summaries.is_empty() && !has_current_pane {
        return vec![ListItem::new("No sessions available")];
    }

    let selected_index = app.target_picker_selected_index().unwrap_or(0);
    let mut items = Vec::new();
    if has_current_pane {
        let marker = if selected_index == 0 { ">" } else { " " };
        items.push(ListItem::new(format!(
            "{marker} Current pane [local] open editor in Grove pane"
        )));
    }
    items.extend(
        app.bridge
            .session_summaries
            .iter()
            .enumerate()
            .map(|(index, session)| {
                let display_index = index + usize::from(has_current_pane);
                let marker = if display_index == selected_index {
                    ">"
                } else {
                    " "
                };
                ListItem::new(session_summary_line(marker, session))
            }),
    );
    items
}

fn session_summary_line(marker: &str, session: &SessionSummary) -> String {
    let role = session
        .role
        .map(|role| match role {
            TargetRole::Ai => "ai",
            TargetRole::Editor => "editor",
            TargetRole::Grove => "grove",
        })
        .unwrap_or("unassigned");
    let location = session.location_hint.as_ref().map_or_else(
        || "unknown location".to_string(),
        |hint| {
            let mut bits = Vec::new();
            if let Some(window_title) = hint.window_title.as_deref() {
                bits.push(window_title.to_string());
            }
            if let Some(tab_title) = hint.tab_title.as_deref() {
                bits.push(tab_title.to_string());
            }
            if bits.is_empty() {
                match (hint.window_id.as_deref(), hint.tab_id.as_deref()) {
                    (Some(window_id), Some(tab_id)) => format!("{window_id}/{tab_id}"),
                    (Some(window_id), None) => window_id.to_string(),
                    (None, Some(tab_id)) => tab_id.to_string(),
                    _ => "unknown location".to_string(),
                }
            } else {
                bits.join(" / ")
            }
        },
    );
    let job = session.job_name.as_deref().unwrap_or("no job");
    format!("{marker} {} [{role}] {job} {location}", session.title)
}

fn title_for_role(role: TargetRole) -> &'static str {
    match role {
        TargetRole::Ai => "Select AI Target",
        TargetRole::Editor => "Select Editor Target",
        TargetRole::Grove => "Select Grove Target",
    }
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
