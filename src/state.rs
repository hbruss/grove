use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::action::ActionDescriptor;
use crate::bridge::protocol::TargetRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Focus {
    Roots,
    #[default]
    Tree,
    Preview,
    PathFilter,
    ContentSearch,
    CommandPalette,
    ContextMenu,
    Dialog,
}

impl Focus {
    pub const fn is_command_surface(self) -> bool {
        matches!(self, Self::CommandPalette)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    #[default]
    Preview,
    Diff,
    Blame,
    Info,
    SearchResults,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedState {
    pub tabs: Vec<PersistedTabState>,
    pub active_tab: usize,
    pub split_ratio: f32,
    pub recent_roots: Vec<PathBuf>,
    pub last_focus: Focus,
    pub last_targets: Option<PersistedTargets>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedTabState {
    pub root: PathBuf,
    pub mode: ContextMode,
    pub expanded_directories: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedTargets {
    pub ai_session_id: Option<String>,
    pub editor_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TargetPickerSelection {
    CurrentPane,
    SessionId(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetPickerState {
    pub role: TargetRole,
    pub selection: TargetPickerSelection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryPickerIntent {
    AddRoot,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryPickerEntryMode {
    DirectoriesOnly,
    FilesOnly,
    FilesAndDirectories,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryPickerEntry {
    pub path: PathBuf,
    pub label: String,
    pub is_parent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryPickerState {
    pub intent: DirectoryPickerIntent,
    pub entry_mode: DirectoryPickerEntryMode,
    pub current_dir: PathBuf,
    pub selected_index: usize,
    pub entries: Vec<DirectoryPickerEntry>,
    pub error_message: Option<String>,
    pub show_hidden: bool,
    pub respect_gitignore: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RootNavigatorState {
    pub selected_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MultiSelectState {
    pub active: bool,
    pub selected_paths: BTreeSet<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptDialogIntent {
    NewFile,
    NewDirectory,
    Rename,
    Duplicate,
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptDialogState {
    pub title: String,
    pub subtitle: Option<String>,
    pub value: String,
    pub confirm_label: String,
    pub intent: PromptDialogIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmDialogIntent {
    OverwriteDestination {
        operation: PromptDialogIntent,
        source_rel: PathBuf,
        dest_rel: PathBuf,
    },
    TrashPath {
        rel_path: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfirmDialogState {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub intent: ConfirmDialogIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DialogState {
    TargetPicker(TargetPickerState),
    DirectoryPicker(DirectoryPickerState),
    Prompt(PromptDialogState),
    Confirm(ConfirmDialogState),
}

#[derive(Debug, Clone, Default)]
pub struct StatusBarState {
    pub severity: StatusSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusSeverity {
    Success,
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GitRepoSummary {
    pub repo_root: PathBuf,
    pub branch_name: String,
    pub staged_paths: usize,
    pub unstaged_paths: usize,
    pub untracked_paths: usize,
    pub conflicted_paths: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverlayState {
    pub command_palette: CommandPaletteState,
    pub context_menu: ContextMenuState,
    pub dialog: Option<DialogState>,
    pub previous_focus: Option<Focus>,
}

impl OverlayState {
    pub const fn command_surface(&self) -> &CommandPaletteState {
        &self.command_palette
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandPaletteState {
    pub query: String,
    pub selected_index: usize,
    pub entries: Vec<ActionDescriptor>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextMenuState {
    pub selected_index: usize,
    pub entries: Vec<ActionDescriptor>,
    pub active: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TimerRegistry {
    pub timers: Vec<TimerRegistration>,
}

#[derive(Debug, Clone)]
pub struct TimerRegistration {
    pub name: String,
    pub deadline: Instant,
}
