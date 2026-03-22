use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use super::theme;
use crate::app::App;
use crate::state::Focus;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let (title, content) = app
        .tabs
        .get(app.active_tab)
        .map(|tab| {
            let base_title = if tab.preview.payload.title.is_empty() {
                match tab.mode {
                    crate::state::ContextMode::Preview => "Preview".to_string(),
                    crate::state::ContextMode::Diff => "Diff".to_string(),
                    crate::state::ContextMode::Blame => "Blame".to_string(),
                    crate::state::ContextMode::Info => "Info".to_string(),
                    crate::state::ContextMode::SearchResults => "Search Results".to_string(),
                }
            } else {
                tab.preview.payload.title.clone()
            };
            let title = if app.focus == Focus::Preview {
                format!("{base_title} *")
            } else {
                base_title
            };
            let content = crate::preview::render::visible_text_from_cache(
                tab.preview.render_cache.as_ref(),
                tab.preview.scroll_row,
                area.height.saturating_sub(2) as usize,
                tab.preview.cursor_line,
                tab.preview.preview_selection_range(),
                app.focus == Focus::Preview,
            );
            (title, content)
        })
        .unwrap_or_else(|| {
            (
                "Preview".to_string(),
                ratatui::text::Text::from("Preview unavailable"),
            )
        });
    let block = theme::panel_block(title.as_str());
    let widget = Paragraph::new(content)
        .style(theme::panel_style())
        .block(block);
    frame.render_widget(widget, area);
}
