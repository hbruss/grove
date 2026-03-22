use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::state::Focus;

use super::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let line = if app.tabs.is_empty() {
        Line::from("No tabs")
    } else {
        let labels = app.tab_display_labels();
        let spans =
            app.tabs
                .iter()
                .enumerate()
                .flat_map(|(index, _)| {
                    let label = labels.get(index).cloned().unwrap_or_else(|| {
                        crate::app::RootDisplayLabel {
                            primary: format!("tab {}", index + 1),
                            disambiguator: None,
                        }
                    });
                    let mut style = theme::panel_style();
                    if index == app.active_tab {
                        style = style
                            .fg(TuiColor::Rgb(232, 236, 242))
                            .bg(TuiColor::Rgb(58, 63, 72))
                            .add_modifier(Modifier::BOLD);
                    } else {
                        style = style.fg(TuiColor::Rgb(154, 164, 180));
                    }
                    let disambiguator_style = if index == app.active_tab {
                        style
                            .fg(TuiColor::Rgb(166, 176, 192))
                            .remove_modifier(Modifier::BOLD)
                    } else {
                        style.fg(TuiColor::Rgb(112, 120, 136))
                    };

                    let mut spans = vec![Span::styled(format!(" {} ", label.primary), style)];
                    if let Some(disambiguator) = label.disambiguator {
                        spans.push(Span::styled(
                            format!("· {} ", disambiguator),
                            disambiguator_style,
                        ));
                    }
                    spans.push(Span::styled(" ", Style::default()));
                    spans
                })
                .collect::<Vec<_>>();
        Line::from(spans)
    };

    let title = if app.focus == Focus::Tabs {
        "Tabs *"
    } else {
        "Tabs"
    };
    let widget = Paragraph::new(line)
        .style(theme::panel_style())
        .block(theme::panel_block(title));
    frame.render_widget(widget, area);
}

pub fn render_context(frame: &mut Frame, area: Rect) {
    let widget = Paragraph::new("Context Tabs")
        .style(theme::panel_style())
        .block(theme::panel_block("Context Tabs"));
    frame.render_widget(widget, area);
}
