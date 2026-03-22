use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::state::Focus;

use super::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let line = if app.bookmark_paths().is_empty() {
        Line::from("No bookmarks")
    } else {
        let selected_index = app.selected_bookmark_index();
        let labels = app.bookmark_display_labels();
        let spans =
            app.bookmark_paths()
                .iter()
                .enumerate()
                .flat_map(|(index, path)| {
                    let label = labels.get(index).cloned().unwrap_or_else(|| {
                        crate::app::RootDisplayLabel {
                            primary: path.display().to_string(),
                            disambiguator: None,
                        }
                    });
                    let selected = app.focus == Focus::Bookmarks && selected_index == Some(index);
                    let active = app.bookmark_is_active(path);
                    let open = app.bookmark_is_open(path);

                    let mut style = theme::panel_style().fg(TuiColor::Rgb(160, 169, 184));
                    if open {
                        style = style.fg(TuiColor::Rgb(141, 197, 222));
                    }
                    if active {
                        style = style
                            .fg(TuiColor::Rgb(147, 208, 170))
                            .add_modifier(Modifier::BOLD);
                    }
                    if selected {
                        style = style.bg(TuiColor::Rgb(58, 63, 72));
                    }
                    let disambiguator_style = if active {
                        style
                            .fg(TuiColor::Rgb(123, 170, 139))
                            .remove_modifier(Modifier::BOLD)
                    } else if open {
                        style.fg(TuiColor::Rgb(104, 145, 164))
                    } else {
                        style.fg(TuiColor::Rgb(112, 120, 136))
                    };

                    let marker = if active {
                        "●"
                    } else if open {
                        "◦"
                    } else {
                        "·"
                    };

                    let mut spans =
                        vec![Span::styled(format!(" {marker} {} ", label.primary), style)];
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

    let widget = Paragraph::new(line)
        .style(theme::panel_style())
        .block(theme::panel_block("Bookmarks"));
    frame.render_widget(widget, area);
}
