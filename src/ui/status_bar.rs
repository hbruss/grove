use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::bridge::protocol::TargetRole;

use super::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let widget = Paragraph::new(status_text(app))
        .style(theme::panel_style())
        .block(theme::panel_block("Status Bar"));
    frame.render_widget(widget, area);
}

fn status_text(app: &App) -> String {
    let bridge = if app.bridge.connected {
        "bridge: online"
    } else {
        "bridge: offline"
    };
    let git = git_status_text(app);
    let ai = format!("AI: {}", app.bridge_target_label(TargetRole::Ai));
    let editor = format!("Editor: {}", app.bridge_target_label(TargetRole::Editor));
    let multi_select = multi_select_text(app);
    let picker = if let Some(picker) = app.target_picker_state() {
        let selected = app
            .target_picker_selected_label()
            .unwrap_or_else(|| "none".to_string());
        format!("Picker: {} -> {}", target_role_label(picker.role), selected)
    } else {
        "Picker: idle".to_string()
    };

    let mut segments = vec![bridge.to_string(), git, ai, editor];
    if let Some(multi_select) = multi_select {
        segments.push(multi_select);
    }

    if app.status.message.is_empty() {
        segments.push(picker);
    } else {
        segments.push(app.status.message.clone());
    }
    segments.join(" | ")
}

fn git_status_text(app: &App) -> String {
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return "git: none".to_string();
    };
    if tab.git.last_error.is_some() {
        return "git: error".to_string();
    }
    let Some(summary) = app.active_git_summary() else {
        return "git: none".to_string();
    };
    format!(
        "git: {} +{} ~{} ?{} !{}",
        summary.branch_name,
        summary.staged_paths,
        summary.unstaged_paths,
        summary.untracked_paths,
        summary.conflicted_paths
    )
}

fn target_role_label(role: TargetRole) -> &'static str {
    match role {
        TargetRole::Ai => "AI",
        TargetRole::Editor => "editor",
        TargetRole::Grove => "grove",
    }
}

fn multi_select_text(app: &App) -> Option<String> {
    let count = app.active_multi_select_count();
    if count == 0 && !app.active_multi_select_mode() {
        return None;
    }
    Some(format!(
        "multi-select: {count} {}",
        if count == 1 { "path" } else { "paths" }
    ))
}
