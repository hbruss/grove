use crate::action::{Action, ActionDescriptor};
use crate::app::App;
use crate::git::backend::{GitChange, GitPathStatus};
use crate::tree::model::NodeKind;

pub fn action_bar_entries(app: &App) -> Vec<ActionDescriptor> {
    vec![
        descriptor(Action::SetAiTarget, "AI", "Ctrl+A", true),
        descriptor(Action::SetEditorTarget, "edit", "Ctrl+E", true),
        descriptor(Action::OpenContentSearch, "search", "Ctrl+F", true),
        descriptor(Action::OpenCommandPalette, "commands", "Ctrl+P", true),
        descriptor(
            Action::TogglePreviewVisibility,
            if app.active_preview_visible() {
                "hide preview"
            } else {
                "show preview"
            },
            "v",
            true,
        ),
        descriptor(
            Action::SetContextModeDiff,
            "diff",
            "d",
            app.active_selection_supports_unstaged_diff(),
        ),
        descriptor(Action::SetContextModePreview, "preview", "p", true),
        descriptor(
            Action::StageSelectedPath,
            "stage",
            "s",
            can_stage_selected_path(app),
        ),
        descriptor(
            Action::UnstageSelectedPath,
            "unstage",
            "u",
            can_unstage_selected_path(app),
        ),
    ]
    .into_iter()
    .filter(|entry| entry.enabled)
    .collect()
}

pub fn command_palette_entries(app: &App, query: &str) -> Vec<ActionDescriptor> {
    let mut entries = Vec::new();
    entries.extend(selection_entries(app));
    entries.extend(root_entries(app));
    entries.extend(git_entries(app));
    entries.extend(target_entries());
    entries.extend(view_entries(app));

    filter_entries(entries, query)
}

pub const fn command_palette_section_label(action: &Action) -> &'static str {
    match action {
        Action::OpenInEditor
        | Action::OpenExternally
        | Action::RevealInFinder
        | Action::SendRelativePath
        | Action::CopyRelativePath
        | Action::CopyAbsolutePath
        | Action::Rename
        | Action::Duplicate
        | Action::Move
        | Action::Trash => "Selection",
        Action::NewTab
        | Action::NewFile
        | Action::NewDirectory
        | Action::PinBookmark
        | Action::UnpinBookmark
        | Action::CloseTab => "Root",
        Action::StageSelectedPath | Action::UnstageSelectedPath => "Git",
        Action::SetAiTarget | Action::SetEditorTarget => "Targets",
        Action::OpenContentSearch
        | Action::TogglePreviewVisibility
        | Action::SetContextModeDiff
        | Action::SetContextModePreview => "View",
        _ => "Commands",
    }
}

fn filter_entries(entries: Vec<ActionDescriptor>, query: &str) -> Vec<ActionDescriptor> {
    let mut entries = entries
        .into_iter()
        .filter(|entry| entry.enabled)
        .collect::<Vec<_>>();
    if query.is_empty() {
        return entries;
    }

    let query = query.to_ascii_lowercase();
    entries.retain(|entry| {
        entry.label.to_ascii_lowercase().contains(&query)
            || entry
                .subtitle
                .as_deref()
                .is_some_and(|subtitle| subtitle.to_ascii_lowercase().contains(&query))
    });
    entries
}

fn selection_entries(app: &App) -> Vec<ActionDescriptor> {
    vec![
        descriptor(
            Action::OpenInEditor,
            "open in editor",
            "Edit selected file",
            has_file_selection(app),
        ),
        descriptor(
            Action::OpenExternally,
            "open externally",
            "System open",
            has_openable_selection(app),
        ),
        descriptor(
            Action::RevealInFinder,
            "reveal in finder",
            "Reveal the selected path in Finder",
            has_non_root_selection(app),
        ),
        descriptor(
            Action::SendRelativePath,
            "send selected paths",
            "Inject selected path(s) into AI target",
            has_non_root_selection(app),
        ),
        descriptor(
            Action::CopyRelativePath,
            "copy relative path",
            "Copy the selected path relative to the root",
            has_non_root_selection(app),
        ),
        descriptor(
            Action::CopyAbsolutePath,
            "copy absolute path",
            "Copy the selected absolute path",
            has_non_root_selection(app),
        ),
        descriptor(
            Action::Rename,
            "rename",
            "Rename the selected path",
            has_non_root_selection(app),
        ),
        descriptor(
            Action::Duplicate,
            "duplicate",
            "Duplicate the selected path",
            has_non_root_selection(app),
        ),
        descriptor(
            Action::Move,
            "move",
            "Move the selected path",
            has_non_root_selection(app),
        ),
        descriptor(
            Action::Trash,
            "trash",
            "Move the selected path to trash",
            has_non_root_selection(app),
        ),
    ]
}

fn root_entries(app: &App) -> Vec<ActionDescriptor> {
    let targets_selected_root = app
        .bookmark_target_root()
        .zip(app.tabs.get(app.active_tab).map(|tab| tab.root.clone()))
        .is_some_and(|(target, active_root)| target != active_root);

    vec![
        descriptor(
            Action::NewTab,
            "open as root tab",
            "Promote the selected directory into a root tab",
            can_open_selected_directory_as_root_tab(app),
        ),
        descriptor(
            Action::NewFile,
            "new file",
            "Create a file under the active root",
            has_active_tab(app),
        ),
        descriptor(
            Action::NewDirectory,
            "new directory",
            "Create a directory under the active root",
            has_active_tab(app),
        ),
        descriptor(
            Action::PinBookmark,
            if targets_selected_root {
                "pin selected root"
            } else {
                "pin active root"
            },
            if targets_selected_root {
                "Pin the selected directory as a root"
            } else {
                "Pin the active root"
            },
            can_pin_bookmark_target(app),
        ),
        descriptor(
            Action::UnpinBookmark,
            if targets_selected_root {
                "unpin selected root"
            } else {
                "unpin active root"
            },
            if targets_selected_root {
                "Unpin the selected root"
            } else {
                "Unpin the active root"
            },
            can_unpin_bookmark_target(app),
        ),
        descriptor(
            Action::CloseTab,
            "close tab",
            "Close the active tab",
            can_close_tab(app),
        ),
    ]
}

fn git_entries(app: &App) -> Vec<ActionDescriptor> {
    vec![
        descriptor(
            Action::StageSelectedPath,
            "stage selected path",
            "Git stage",
            can_stage_selected_path(app),
        ),
        descriptor(
            Action::UnstageSelectedPath,
            "unstage selected path",
            "Git unstage",
            can_unstage_selected_path(app),
        ),
    ]
}

fn target_entries() -> Vec<ActionDescriptor> {
    vec![
        descriptor(Action::SetAiTarget, "AI target", "Pick the AI pane", true),
        descriptor(
            Action::SetEditorTarget,
            "editor target",
            "Pick the editor pane",
            true,
        ),
    ]
}

fn view_entries(app: &App) -> Vec<ActionDescriptor> {
    vec![
        descriptor(Action::OpenContentSearch, "search", "Content search", true),
        descriptor(
            Action::TogglePreviewVisibility,
            if app.active_preview_visible() {
                "hide preview"
            } else {
                "show preview"
            },
            "Toggle preview pane visibility",
            true,
        ),
        descriptor(
            Action::SetContextModeDiff,
            "diff",
            "Right-panel diff mode",
            app.active_selection_supports_unstaged_diff(),
        ),
        descriptor(
            Action::SetContextModePreview,
            "preview",
            "Right-panel preview mode",
            true,
        ),
    ]
}

fn descriptor(action: Action, label: &str, subtitle: &str, enabled: bool) -> ActionDescriptor {
    ActionDescriptor {
        action,
        label: label.to_string(),
        subtitle: Some(subtitle.to_string()),
        enabled,
    }
}

fn has_file_selection(app: &App) -> bool {
    selected_kind(app).is_some_and(|kind| matches!(kind, NodeKind::File | NodeKind::SymlinkFile))
}

fn has_active_tab(app: &App) -> bool {
    app.tabs.get(app.active_tab).is_some()
}

fn has_openable_selection(app: &App) -> bool {
    has_non_root_selection(app)
}

fn has_non_root_selection(app: &App) -> bool {
    app.tabs
        .get(app.active_tab)
        .and_then(|tab| tab.tree.selected_rel_path())
        .is_some_and(|path| !path.as_os_str().is_empty())
}

fn can_stage_selected_path(app: &App) -> bool {
    selected_git_path_status(app).is_some_and(|status| {
        !status.conflicted
            && !status.ignored
            && (status.untracked || status.worktree != GitChange::Unmodified)
    })
}

fn can_unstage_selected_path(app: &App) -> bool {
    selected_git_path_status(app)
        .is_some_and(|status| !status.conflicted && status.index != GitChange::Unmodified)
}

fn can_pin_bookmark_target(app: &App) -> bool {
    app.bookmark_target_root()
        .is_some_and(|target| !app.bookmark_paths().iter().any(|path| path == &target))
}

fn can_unpin_bookmark_target(app: &App) -> bool {
    app.bookmark_target_root()
        .is_some_and(|target| app.bookmark_paths().iter().any(|path| path == &target))
}

fn can_close_tab(app: &App) -> bool {
    app.tabs.len() > 1
}

fn can_open_selected_directory_as_root_tab(app: &App) -> bool {
    let Some(root_candidate) = app.selected_directory_root_candidate() else {
        return false;
    };
    app.tabs
        .get(app.active_tab)
        .is_some_and(|tab| tab.root != root_candidate)
}

fn selected_git_path_status(app: &App) -> Option<GitPathStatus> {
    app.active_selected_git_path_status()
}

fn selected_kind(app: &App) -> Option<NodeKind> {
    let tab = app.tabs.get(app.active_tab)?;
    let row = tab.tree.visible_rows.get(tab.tree.selected_row)?;
    tab.tree.node(row.node_id).map(|node| node.kind.clone())
}
