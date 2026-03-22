use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use super::theme;
use crate::app::{App, PathIndexStatus};
use crate::state::Focus;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let content = app
        .tabs
        .get(app.active_tab)
        .map(|tab| {
            let base = if tab.path_filter.query.is_empty() {
                if app.focus == Focus::PathFilter {
                    "/".to_string()
                } else {
                    "Press / to filter".to_string()
                }
            } else {
                tab.path_filter.query.clone()
            };

            let visibility = format!(
                "[H {} G {}]",
                on_off(tab.tree.show_hidden),
                on_off(tab.tree.respect_gitignore)
            );

            match &tab.path_index.status {
                PathIndexStatus::Building { indexed_paths } => {
                    format!("{base} {visibility} [indexing {indexed_paths}]")
                }
                PathIndexStatus::Error(message) => {
                    format!("{base} {visibility} [index error: {message}]")
                }
                PathIndexStatus::Ready | PathIndexStatus::Idle => format!("{base} {visibility}"),
            }
        })
        .unwrap_or_else(|| "Press / to filter".to_string());

    let widget = Paragraph::new(content)
        .style(theme::panel_style())
        .block(theme::panel_block("Path Filter"));
    frame.render_widget(widget, area);
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
