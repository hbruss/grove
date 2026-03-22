use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::widgets::{Block, Borders};

use crate::git::backend::GitStatus;
use crate::tree::model::NodeKind;

pub fn panel_block<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(TuiColor::Rgb(82, 89, 101)))
}

pub fn panel_style() -> Style {
    Style::default().fg(TuiColor::Rgb(214, 219, 230))
}

pub fn preview_path_style() -> Style {
    Style::default().fg(TuiColor::Rgb(128, 138, 156))
}

pub fn preview_header_band_style() -> Style {
    Style::default().bg(TuiColor::Rgb(34, 39, 46))
}

pub fn directory_picker_header_band_style() -> Style {
    Style::default().bg(TuiColor::Rgb(30, 34, 41))
}

pub fn directory_picker_path_label_style() -> Style {
    Style::default().fg(TuiColor::Rgb(118, 128, 144))
}

pub fn directory_picker_path_value_style() -> Style {
    Style::default()
        .fg(TuiColor::Rgb(232, 236, 242))
        .add_modifier(Modifier::BOLD)
}

pub fn directory_picker_toggle_label_style() -> Style {
    Style::default().fg(TuiColor::Rgb(118, 128, 144))
}

pub fn directory_picker_toggle_on_style() -> Style {
    Style::default()
        .fg(TuiColor::Rgb(122, 196, 110))
        .add_modifier(Modifier::BOLD)
}

pub fn directory_picker_toggle_off_style() -> Style {
    Style::default().fg(TuiColor::Rgb(102, 111, 126))
}

pub fn directory_picker_error_style() -> Style {
    Style::default()
        .fg(TuiColor::Rgb(232, 117, 111))
        .add_modifier(Modifier::BOLD)
}

pub fn directory_picker_footer_style() -> Style {
    Style::default().fg(TuiColor::Rgb(146, 155, 168))
}

pub fn directory_picker_hint_style() -> Style {
    Style::default().fg(TuiColor::Rgb(214, 219, 230))
}

pub fn directory_picker_item_style(selected: bool) -> Style {
    let style = Style::default().fg(TuiColor::Rgb(214, 219, 230));
    apply_directory_picker_selection(style, selected)
}

pub fn directory_picker_item_icon_style(selected: bool) -> Style {
    let style = Style::default().fg(TuiColor::Rgb(214, 198, 145));
    apply_directory_picker_selection(style, selected)
}

pub fn directory_picker_parent_item_style(selected: bool) -> Style {
    let style = Style::default()
        .fg(TuiColor::Rgb(141, 197, 222))
        .add_modifier(Modifier::DIM);
    apply_directory_picker_selection(style, selected)
}

pub fn directory_picker_item_background_style(selected: bool) -> Style {
    apply_directory_picker_selection(Style::default().fg(TuiColor::Rgb(214, 219, 230)), selected)
}

pub fn preview_diff_added_line_style() -> Style {
    Style::default()
        .bg(TuiColor::Rgb(46, 86, 64))
        .fg(TuiColor::Rgb(227, 247, 233))
}

pub fn preview_diff_removed_line_style() -> Style {
    Style::default()
        .bg(TuiColor::Rgb(94, 54, 60))
        .fg(TuiColor::Rgb(250, 228, 232))
}

pub fn preview_diff_hunk_header_style() -> Style {
    Style::default()
        .bg(TuiColor::Rgb(42, 47, 58))
        .fg(TuiColor::Rgb(164, 176, 194))
}

pub fn preview_metadata_label_style() -> Style {
    Style::default().fg(TuiColor::Rgb(118, 128, 144))
}

pub fn preview_metadata_value_style() -> Style {
    Style::default()
        .fg(TuiColor::Rgb(232, 236, 242))
        .add_modifier(Modifier::BOLD)
}

pub fn preview_selected_line_style(focused: bool) -> Style {
    let style = Style::default()
        .bg(TuiColor::Rgb(66, 76, 92))
        .fg(TuiColor::Rgb(228, 233, 242));
    if focused {
        style
    } else {
        style.fg(TuiColor::Rgb(214, 219, 230))
    }
}

pub fn preview_cursor_line_style(focused: bool) -> Style {
    let style = Style::default()
        .bg(TuiColor::Rgb(82, 95, 118))
        .fg(TuiColor::Rgb(242, 246, 252));
    if focused {
        style
    } else {
        style.fg(TuiColor::Rgb(226, 232, 240))
    }
}

pub fn command_palette_section_style() -> Style {
    Style::default()
        .fg(TuiColor::Rgb(118, 128, 144))
        .add_modifier(Modifier::BOLD)
}

pub fn command_palette_subtitle_style() -> Style {
    Style::default().fg(TuiColor::Rgb(146, 155, 168))
}

pub fn git_summary_branch_style() -> Style {
    Style::default()
        .fg(TuiColor::Rgb(166, 196, 255))
        .add_modifier(Modifier::BOLD)
}

pub fn git_summary_label_style() -> Style {
    Style::default().fg(TuiColor::Rgb(118, 128, 144))
}

pub fn git_summary_value_style(status: GitStatus) -> Style {
    Style::default().fg(git_status_color(status))
}

pub fn tree_selection_accent_style(selected: bool, batched: bool) -> Style {
    apply_tree_row_state(
        Style::default()
            .fg(TuiColor::Rgb(96, 168, 240))
            .add_modifier(Modifier::BOLD),
        selected,
        batched,
    )
}

pub fn tree_batch_marker_style(selected: bool, batched: bool) -> Style {
    let style = if batched {
        Style::default()
            .fg(TuiColor::Rgb(132, 198, 166))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TuiColor::Rgb(92, 102, 118))
    };
    apply_tree_row_state(style, selected, batched)
}

pub fn tree_indent_style(depth: u16, selected: bool, batched: bool) -> Style {
    let palette = [
        TuiColor::Rgb(72, 84, 98),
        TuiColor::Rgb(76, 89, 104),
        TuiColor::Rgb(82, 94, 108),
        TuiColor::Rgb(86, 98, 114),
    ];
    apply_tree_row_state(
        Style::default().fg(palette[depth as usize % palette.len()]),
        selected,
        batched,
    )
}

pub fn tree_disclosure_style(selected: bool, batched: bool) -> Style {
    apply_tree_row_state(
        Style::default().fg(TuiColor::Rgb(118, 128, 144)),
        selected,
        batched,
    )
}

pub fn tree_icon_style(
    kind: &NodeKind,
    depth: u16,
    hidden: bool,
    selected: bool,
    batched: bool,
) -> Style {
    let mut style = Style::default().fg(match kind {
        NodeKind::Directory => depth_palette(depth, true),
        NodeKind::SymlinkDirectory => TuiColor::Rgb(130, 193, 219),
        NodeKind::SymlinkFile => TuiColor::Rgb(170, 178, 192),
        NodeKind::File => TuiColor::Rgb(204, 210, 220),
    });
    if hidden {
        style = style.add_modifier(Modifier::DIM);
    }
    apply_tree_row_state(style, selected, batched)
}

pub fn tree_name_style(
    kind: &NodeKind,
    depth: u16,
    hidden: bool,
    selected: bool,
    batched: bool,
) -> Style {
    let mut style = Style::default().fg(match kind {
        NodeKind::Directory | NodeKind::SymlinkDirectory => depth_palette(depth, true),
        NodeKind::File | NodeKind::SymlinkFile => depth_palette(depth, false),
    });
    if matches!(kind, NodeKind::Directory | NodeKind::SymlinkDirectory) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if hidden {
        style = style.add_modifier(Modifier::DIM);
    }
    apply_tree_row_state(style, selected, batched)
}

pub fn tree_git_dot_style(status: GitStatus, selected: bool, batched: bool) -> Style {
    apply_tree_row_state(
        Style::default().fg(git_status_color(status)),
        selected,
        batched,
    )
}

fn depth_palette(depth: u16, directory: bool) -> TuiColor {
    let directory_palette = [
        TuiColor::Rgb(214, 198, 145),
        TuiColor::Rgb(141, 197, 222),
        TuiColor::Rgb(147, 208, 170),
        TuiColor::Rgb(214, 177, 138),
        TuiColor::Rgb(194, 168, 228),
    ];
    let file_palette = [
        TuiColor::Rgb(222, 226, 232),
        TuiColor::Rgb(198, 213, 228),
        TuiColor::Rgb(204, 220, 210),
        TuiColor::Rgb(226, 214, 196),
        TuiColor::Rgb(214, 206, 232),
    ];
    let palette = if directory {
        directory_palette
    } else {
        file_palette
    };
    palette[depth as usize % palette.len()]
}

fn apply_tree_row_state(style: Style, selected: bool, batched: bool) -> Style {
    if selected {
        style.bg(TuiColor::Rgb(58, 63, 72))
    } else if batched {
        style.bg(TuiColor::Rgb(44, 49, 57))
    } else {
        style
    }
}

fn apply_directory_picker_selection(style: Style, selected: bool) -> Style {
    if selected {
        style.bg(TuiColor::Rgb(58, 63, 72))
    } else {
        style
    }
}

fn git_status_color(status: GitStatus) -> TuiColor {
    match status {
        GitStatus::Unmodified => TuiColor::Rgb(92, 102, 118),
        GitStatus::Modified | GitStatus::Typechange | GitStatus::Renamed => {
            TuiColor::Rgb(226, 192, 104)
        }
        GitStatus::Added => TuiColor::Rgb(122, 196, 110),
        GitStatus::Deleted => TuiColor::Rgb(232, 117, 111),
        GitStatus::Conflicted => TuiColor::Rgb(220, 130, 204),
        GitStatus::Ignored => TuiColor::Rgb(102, 111, 126),
        GitStatus::Unknown => TuiColor::Rgb(120, 174, 255),
    }
}

pub fn markdown_body_style() -> Style {
    Style::default().fg(TuiColor::Rgb(220, 225, 234))
}

pub fn markdown_heading_style(level: u8) -> Style {
    match level {
        1 => Style::default()
            .fg(TuiColor::Rgb(138, 198, 255))
            .add_modifier(Modifier::BOLD),
        2 => Style::default()
            .fg(TuiColor::Rgb(232, 206, 124))
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(TuiColor::Rgb(236, 239, 244))
            .add_modifier(Modifier::BOLD),
    }
}

pub fn markdown_quote_style() -> Style {
    Style::default().fg(TuiColor::Rgb(150, 158, 170))
}

pub fn markdown_code_block_style() -> Style {
    Style::default()
        .fg(TuiColor::Rgb(236, 239, 244))
        .bg(TuiColor::Rgb(28, 31, 37))
}

pub fn markdown_rule_style() -> Style {
    Style::default().fg(TuiColor::Rgb(92, 101, 114))
}
