use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::theme;
use crate::app::App;
use crate::git::backend::{GitStatus, git_status_for_path};
use crate::state::Focus;
use crate::state::GitRepoSummary;
use crate::tree::model::{Node, NodeKind};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = Vec::new();
    if let Some(tab) = app.tabs.get(app.active_tab) {
        let repo_summary = tab.git.repo_summary();
        if let Some(summary) = repo_summary.as_ref() {
            lines.push(render_repo_strip(summary));
        }
        let row_budget =
            area.height
                .saturating_sub(2 + u16::from(repo_summary.is_some())) as usize;
        for (idx, row) in tab
            .tree
            .visible_rows
            .iter()
            .enumerate()
            .skip(tab.tree.scroll_row)
            .take(row_budget)
        {
            if let Some(node) = tab.tree.node(row.node_id) {
                let batched = tab.multi_select.selected_paths.contains(&node.rel_path);
                lines.push(render_tree_row(
                    tab,
                    node,
                    row.depth,
                    idx == tab.tree.selected_row,
                    batched,
                ));
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from("(empty)"));
    }

    let title = if app.active_multi_select_mode() {
        if app.focus == Focus::Tree {
            "Tree [multi] *"
        } else {
            "Tree [multi]"
        }
    } else if app.focus == Focus::Tree {
        "Tree *"
    } else {
        "Tree"
    };
    let widget = Paragraph::new(Text::from(lines))
        .style(theme::panel_style())
        .block(theme::panel_block(title));
    frame.render_widget(widget, area);
}

fn render_tree_row(
    tab: &crate::app::TabState,
    node: &Node,
    depth: u16,
    selected: bool,
    batched: bool,
) -> Line<'static> {
    let git_status = tab
        .git
        .status_map
        .get(&node.rel_path)
        .map(git_status_for_path)
        .unwrap_or(node.git);
    let mut spans = Vec::new();
    spans.push(Span::styled(
        if selected { "▎" } else { " " },
        theme::tree_selection_accent_style(selected, batched),
    ));
    spans.push(Span::styled(
        if batched { "●" } else { " " },
        theme::tree_batch_marker_style(selected, batched),
    ));
    spans.push(Span::styled(
        " ",
        theme::tree_selection_accent_style(selected, batched),
    ));

    for indent_depth in 0..depth {
        spans.push(Span::styled(
            "│ ",
            theme::tree_indent_style(indent_depth, selected, batched),
        ));
    }

    if matches!(node.kind, NodeKind::Directory | NodeKind::SymlinkDirectory) {
        spans.push(Span::styled(
            if node.expanded { " " } else { " " },
            theme::tree_disclosure_style(selected, batched),
        ));
    } else {
        spans.push(Span::styled(
            "  ",
            theme::tree_disclosure_style(selected, batched),
        ));
    }

    spans.push(Span::styled(
        format!("{} ", tree_icon(node)),
        theme::tree_icon_style(&node.kind, depth, node.is_hidden, selected, batched),
    ));
    if git_status != GitStatus::Unmodified {
        spans.push(Span::styled(
            " ",
            theme::tree_git_dot_style(git_status, selected, batched),
        ));
    } else {
        spans.push(Span::styled(
            "  ",
            theme::tree_git_dot_style(git_status, selected, batched),
        ));
    }
    spans.push(Span::styled(
        node.name.clone(),
        theme::tree_name_style(&node.kind, depth, node.is_hidden, selected, batched),
    ));

    Line::from(spans)
}

fn render_repo_strip(summary: &GitRepoSummary) -> Line<'static> {
    let mut spans = vec![
        Span::styled(" ", theme::git_summary_branch_style()),
        Span::styled(
            summary.branch_name.clone(),
            theme::git_summary_branch_style(),
        ),
        Span::raw(" "),
    ];
    push_repo_count(
        &mut spans,
        "+",
        summary.staged_paths,
        "staged",
        GitStatus::Added,
    );
    spans.push(Span::raw(" "));
    push_repo_count(
        &mut spans,
        "~",
        summary.unstaged_paths,
        "unstaged",
        GitStatus::Modified,
    );
    spans.push(Span::raw(" "));
    push_repo_count(
        &mut spans,
        "?",
        summary.untracked_paths,
        "untracked",
        GitStatus::Unknown,
    );
    spans.push(Span::raw(" "));
    push_repo_count(
        &mut spans,
        "!",
        summary.conflicted_paths,
        "conflict",
        GitStatus::Conflicted,
    );
    Line::from(spans)
}

fn push_repo_count(
    spans: &mut Vec<Span<'static>>,
    prefix: &str,
    count: usize,
    label: &str,
    status: GitStatus,
) {
    spans.push(Span::styled(
        format!("{prefix}{count}"),
        theme::git_summary_value_style(status),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        label.to_string(),
        theme::git_summary_label_style(),
    ));
}

fn tree_icon(node: &Node) -> &'static str {
    match node.kind {
        NodeKind::Directory => {
            if node.expanded {
                ""
            } else {
                ""
            }
        }
        NodeKind::SymlinkDirectory => "",
        NodeKind::SymlinkFile => "",
        NodeKind::File => file_icon_for_name(&node.name),
    }
}

fn file_icon_for_name(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower == ".gitignore" {
        return "";
    }
    if lower == "cargo.toml" {
        return "";
    }
    if lower == "cargo.lock" {
        return "󰌾";
    }
    if lower == ".env" || lower.starts_with(".env.") {
        return "";
    }

    match lower.rsplit('.').next() {
        Some("md") | Some("markdown") => "",
        Some("json") => "",
        Some("toml") => "",
        Some("yaml") | Some("yml") => "",
        Some("rs") => "",
        Some("js") | Some("mjs") | Some("cjs") => "",
        Some("ts") => "",
        Some("tsx") | Some("jsx") => "",
        Some("html") | Some("htm") => "",
        Some("css") => "",
        Some("sh") => "",
        Some("py") => "",
        Some("lock") => "󰌾",
        Some("svg") => "󰜡",
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") => "󰈟",
        _ => "",
    }
}
