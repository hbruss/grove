use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::app::App;
use crate::state::{DialogState, DirectoryPickerEntry, DirectoryPickerState};

use super::{target_picker, theme};

pub fn render(frame: &mut Frame, app: &App) {
    let Some(dialog) = app.dialog_state() else {
        return;
    };

    match dialog {
        DialogState::TargetPicker(_) => target_picker::render(frame, app),
        DialogState::DirectoryPicker(state) => render_directory_picker(frame, state),
        DialogState::Prompt(state) => {
            let area = centered_rect(60, 28, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title(state.title.as_str())
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

            let subtitle = state.subtitle.as_deref().unwrap_or_default();
            frame.render_widget(
                Paragraph::new(subtitle).style(theme::panel_style()),
                layout[0],
            );
            frame.render_widget(
                Paragraph::new(format!("Value: {}", state.value)).style(theme::panel_style()),
                layout[1],
            );
            frame.render_widget(
                Paragraph::new(format!("Enter {}  Esc cancel", state.confirm_label))
                    .style(theme::panel_style()),
                layout[2],
            );
        }
        DialogState::Confirm(state) => {
            let area = centered_rect(56, 24, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title(state.title.as_str())
                .borders(Borders::ALL)
                .style(theme::panel_style());
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(2), Constraint::Length(2)])
                .split(inner);

            frame.render_widget(
                Paragraph::new(state.message.as_str()).style(theme::panel_style()),
                layout[0],
            );
            frame.render_widget(
                Paragraph::new(format!("Enter {}  Esc cancel", state.confirm_label))
                    .style(theme::panel_style()),
                layout[1],
            );
        }
    }
}

fn render_directory_picker(frame: &mut Frame, state: &DirectoryPickerState) {
    let area = centered_rect(76, 68, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Add Root")
        .borders(Borders::ALL)
        .style(theme::panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(directory_picker_header_text(state))
            .style(theme::directory_picker_header_band_style()),
        layout[0],
    );

    let error_text = state.error_message.as_deref().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(error_text).style(theme::directory_picker_error_style()),
        layout[1],
    );

    let items = directory_picker_items(state);
    let list = List::new(items)
        .style(theme::panel_style())
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("▎ ");
    let mut list_state = ListState::default();
    if !state.entries.is_empty() {
        list_state.select(Some(state.selected_index.min(state.entries.len() - 1)));
    }
    frame.render_stateful_widget(list, layout[2], &mut list_state);

    frame.render_widget(
        Paragraph::new(directory_picker_footer_text())
            .style(theme::directory_picker_footer_style()),
        layout[3],
    );
}

fn directory_picker_header_text(state: &DirectoryPickerState) -> Text<'static> {
    let hidden_state = if state.show_hidden {
        ("on", theme::directory_picker_toggle_on_style())
    } else {
        ("off", theme::directory_picker_toggle_off_style())
    };
    let gitignore_state = if state.respect_gitignore {
        ("on", theme::directory_picker_toggle_on_style())
    } else {
        ("off", theme::directory_picker_toggle_off_style())
    };

    Text::from(vec![
        Line::from(vec![
            Span::styled("Path: ", theme::directory_picker_path_label_style()),
            Span::styled(
                ellipsize_path_tail(&state.current_dir, 72),
                theme::directory_picker_path_value_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("H ", theme::directory_picker_toggle_label_style()),
            Span::styled(hidden_state.0, hidden_state.1),
            Span::raw("  "),
            Span::styled("G ", theme::directory_picker_toggle_label_style()),
            Span::styled(gitignore_state.0, gitignore_state.1),
            Span::raw("  "),
            Span::styled(
                "Directories only",
                theme::directory_picker_path_label_style(),
            ),
        ]),
    ])
}

fn ellipsize_path_tail(path: &std::path::Path, max_chars: usize) -> String {
    let rendered = path.display().to_string();
    let char_count = rendered.chars().count();
    if char_count <= max_chars {
        return rendered;
    }

    let tail = rendered
        .chars()
        .skip(char_count.saturating_sub(max_chars.saturating_sub(1)))
        .collect::<String>();
    format!("…{tail}")
}

fn directory_picker_items(state: &DirectoryPickerState) -> Vec<ListItem<'static>> {
    state
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let selected = index == state.selected_index;
            render_directory_picker_entry(entry, selected)
        })
        .collect()
}

fn render_directory_picker_entry(
    entry: &DirectoryPickerEntry,
    selected: bool,
) -> ListItem<'static> {
    let icon = if entry.is_parent { "" } else { "" };
    let label_style = if entry.is_parent {
        theme::directory_picker_parent_item_style(selected)
    } else {
        theme::directory_picker_item_style(selected)
    };
    let icon_style = theme::directory_picker_item_icon_style(selected);
    let line = Line::from(vec![
        Span::styled(format!("{icon} "), icon_style),
        Span::styled(entry.label.clone(), label_style),
    ]);
    ListItem::new(line)
}

fn directory_picker_footer_text() -> Text<'static> {
    Text::from(Line::from(vec![
        Span::styled("Up/Down select  ", theme::directory_picker_hint_style()),
        Span::styled("Left parent  ", theme::directory_picker_hint_style()),
        Span::styled("Right open folder  ", theme::directory_picker_hint_style()),
        Span::styled("Enter pin + open  ", theme::directory_picker_hint_style()),
        Span::styled("Esc cancel", theme::directory_picker_hint_style()),
    ]))
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
