use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use super::theme;
use crate::app::App;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 2 || area.height < 2 {
        return;
    }

    let content = app
        .tabs
        .get(app.active_tab)
        .map(|tab| {
            crate::preview::render::metadata_text(
                &tab.preview.payload.header,
                area.width.saturating_sub(2),
            )
        })
        .unwrap_or_else(|| ratatui::text::Text::from("Metadata unavailable"));

    let widget = Paragraph::new(content)
        .style(theme::panel_style())
        .block(theme::panel_block("Metadata"));
    frame.render_widget(widget, area);
}
