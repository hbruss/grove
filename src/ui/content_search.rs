use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;

use super::theme;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return;
    };
    if !tab.content_search.active {
        return;
    }

    let area = centered_rect(64, 24, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Content Search")
        .borders(Borders::ALL)
        .style(theme::panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(inner);

    let query = if tab.content_search.query.is_empty() {
        "Query: ".to_string()
    } else {
        format!("Query: {}", tab.content_search.query)
    };
    frame.render_widget(Paragraph::new(query).style(theme::panel_style()), layout[0]);

    let status = tab
        .content_search
        .status_message
        .as_deref()
        .unwrap_or("type a query and press Enter");
    frame.render_widget(
        Paragraph::new(format!("Status: {status}")).style(theme::panel_style()),
        layout[1],
    );

    frame.render_widget(
        Paragraph::new("Enter search/open  Esc close").style(theme::panel_style()),
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
