use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::{App, RootNavigatorEntry, RootNavigatorSection};
use crate::state::Focus;

use super::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let entries = app.root_navigator_entries();
    let selected_index = app.selected_root_index();
    let pinned_entries = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.section == RootNavigatorSection::Pinned)
        .collect::<Vec<_>>();
    let open_entries = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.section == RootNavigatorSection::Open)
        .collect::<Vec<_>>();
    let mut lines = Vec::new();
    let mut selected_line = None;

    if !pinned_entries.is_empty() {
        render_section(
            &mut lines,
            &mut selected_line,
            "Pinned",
            pinned_entries,
            app.focus == Focus::Roots,
            selected_index,
        );
    }
    if !open_entries.is_empty() {
        render_section(
            &mut lines,
            &mut selected_line,
            "Open",
            open_entries,
            app.focus == Focus::Roots,
            selected_index,
        );
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no roots yet".to_string(),
            Style::default()
                .fg(TuiColor::Rgb(98, 108, 124))
                .add_modifier(Modifier::DIM),
        )));
    }

    let title = if app.focus == Focus::Roots {
        "Roots *"
    } else {
        "Roots"
    };
    let scroll = roots_scroll_offset(
        selected_line,
        area.height.saturating_sub(2) as usize,
        lines.len(),
    );
    let widget = Paragraph::new(Text::from(lines))
        .scroll((scroll as u16, 0))
        .style(theme::panel_style())
        .block(theme::panel_block(title));
    frame.render_widget(widget, area);
}

fn render_section<'a, I>(
    lines: &mut Vec<Line<'static>>,
    selected_line: &mut Option<usize>,
    title: &str,
    entries: I,
    roots_focused: bool,
    selected_index: Option<usize>,
) where
    I: IntoIterator<Item = (usize, &'a RootNavigatorEntry)>,
{
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(TuiColor::Rgb(118, 128, 144))
            .add_modifier(Modifier::BOLD),
    )));

    for (index, entry) in entries {
        if roots_focused && selected_index == Some(index) {
            *selected_line = Some(lines.len());
        }
        lines.push(render_entry_line(
            entry,
            roots_focused && selected_index == Some(index),
        ));
    }
}

fn render_entry_line(entry: &RootNavigatorEntry, selected: bool) -> Line<'static> {
    let mut base_style = Style::default().fg(TuiColor::Rgb(162, 171, 186));
    if entry.open {
        base_style = base_style.fg(TuiColor::Rgb(141, 197, 222));
    }
    if entry.active {
        base_style = base_style
            .fg(TuiColor::Rgb(232, 236, 242))
            .add_modifier(Modifier::BOLD);
    }
    if selected {
        base_style = base_style.bg(TuiColor::Rgb(58, 63, 72));
    }

    let marker = if entry.pinned { "󰐃" } else { "◦" };
    let mut spans = vec![Span::styled(
        format!("  {marker} {} ", entry.label),
        base_style,
    )];

    if let Some(disambiguator) = &entry.disambiguator {
        let mut style = Style::default().fg(TuiColor::Rgb(112, 120, 136));
        if entry.active {
            style = style.fg(TuiColor::Rgb(166, 176, 192));
        } else if entry.open {
            style = style.fg(TuiColor::Rgb(104, 145, 164));
        }
        if selected {
            style = style.bg(TuiColor::Rgb(58, 63, 72));
        }
        spans.push(Span::styled(format!("· {} ", disambiguator), style));
    }
    Line::from(spans)
}

fn roots_scroll_offset(
    selected_line: Option<usize>,
    viewport_height: usize,
    total_lines: usize,
) -> usize {
    if viewport_height == 0 || total_lines <= viewport_height {
        return 0;
    }

    let Some(selected_line) = selected_line else {
        return 0;
    };
    selected_line
        .saturating_add(1)
        .saturating_sub(viewport_height)
        .min(total_lines.saturating_sub(viewport_height))
}
