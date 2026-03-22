use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::bridge::protocol::{SessionSummary, TargetResolution};
use crate::config::Config;
use crate::debug_log;
use crate::error::Result;
use crate::git::backend::{
    DiffMode, GitBackend, GitChange, GitPathStatus, GitStatus, LibgitBackend, RepoHandle,
    combine_git_status, git_status_for_path,
};
use crate::preview::image::{
    ImageInlineImage, ImageRenderKey, ImageRenderOutcome, ImageRenderResponse, ImageRenderWorker,
    build_render_request as build_image_render_request, start_background_image_render,
};
use crate::preview::mermaid::{
    MermaidInlineImage, MermaidRenderKey, MermaidRenderOutcome, MermaidRenderResponse,
    MermaidRenderWorker, build_render_request, discover_renderers, start_background_mermaid_render,
};
use crate::preview::model::{
    GitGeneration, PreviewGeneration, PreviewHeader, PreviewPayload, PreviewPresentation,
    PreviewSource, SearchGeneration, SearchPayload,
};
use crate::preview::render::PreviewRenderCache;
use crate::search::SearchResponse;
use crate::search::content::{
    ContentSearchRequest, ContentSearchWorker, start_background_content_search,
};
use crate::state::{
    ConfirmDialogState, ContextMode, DialogState, DirectoryPickerEntry, DirectoryPickerEntryMode,
    DirectoryPickerIntent, DirectoryPickerState, Focus, MultiSelectState, OverlayState,
    PersistedState, PromptDialogState, RootNavigatorState, StatusBarState, StatusSeverity,
    TargetPickerSelection, TargetPickerState, TimerRegistry,
};
use crate::tree::indexer::{self, PathIndexEntry, PathIndexEvent, PathIndexSnapshot};
use crate::tree::model::{Node, NodeKind, TreeState, VisibilitySettings};
use crate::watcher::RefreshPlan;

pub use crate::state::GitRepoSummary;

const DEFAULT_CONTENT_SEARCH_MAX_RESULTS: usize = 200;
const LOG_PATH_INDEX_POLL_THRESHOLD_MS: u128 = 8;
const MAX_PATH_INDEX_BATCHES_PER_POLL: usize = 8;
const SLOW_PATH_INDEX_BATCH_THRESHOLD_MS: u128 = 16;
const DIFF_UNAVAILABLE_MESSAGE: &str = "diff unavailable: select a modified or untracked file";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootDisplayLabel {
    pub primary: String,
    pub disambiguator: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewOverlayPlacement {
    pub x: u16,
    pub y: u16,
    pub width_cells: u16,
    pub height_lines: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootNavigatorSection {
    Pinned,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootNavigatorEntry {
    pub path: PathBuf,
    pub section: RootNavigatorSection,
    pub label: String,
    pub disambiguator: Option<String>,
    pub pinned: bool,
    pub open: bool,
    pub active: bool,
}

impl RootDisplayLabel {
    pub fn text(&self) -> String {
        match &self.disambiguator {
            Some(disambiguator) => format!("{} · {}", self.primary, disambiguator),
            None => self.primary.clone(),
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub state: PersistedState,
    pub roots: RootNavigatorState,
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub focus: Focus,
    pub status: StatusBarState,
    pub overlays: OverlayState,
    pub bridge: BridgeState,
    pub timers: TimerRegistry,
    pub last_preview_overlay: Option<PreviewOverlayPlacement>,
    pub should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            config: Config::default(),
            config_path: None,
            state: PersistedState::default(),
            roots: RootNavigatorState::default(),
            tabs: vec![TabState::default()],
            active_tab: 0,
            focus: Focus::Tree,
            status: StatusBarState::default(),
            overlays: OverlayState::default(),
            bridge: BridgeState::default(),
            timers: TimerRegistry::default(),
            last_preview_overlay: None,
            should_quit: false,
        }
    }
}

impl App {
    pub fn new_with_config(config: Config, config_path: PathBuf) -> Self {
        let mut app = Self::default();
        let visibility = VisibilitySettings {
            show_hidden: config.general.show_hidden,
            respect_gitignore: config.general.respect_gitignore,
        };
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut tab = TabState::new_with_visibility(root, visibility);
        tab.split_ratio = config.layout.split_ratio;
        app.state.split_ratio = config.layout.split_ratio;
        app.config = config;
        app.config_path = Some(config_path);
        app.tabs = vec![tab];
        app
    }

    pub fn set_config_path(&mut self, config_path: PathBuf) {
        self.config_path = Some(config_path);
    }

    pub fn dialog_state(&self) -> Option<&DialogState> {
        self.overlays.dialog.as_ref()
    }

    pub fn target_picker_state(&self) -> Option<&TargetPickerState> {
        match self.overlays.dialog.as_ref() {
            Some(DialogState::TargetPicker(state)) => Some(state),
            _ => None,
        }
    }

    fn target_picker_state_mut(&mut self) -> Option<&mut TargetPickerState> {
        match self.overlays.dialog.as_mut() {
            Some(DialogState::TargetPicker(state)) => Some(state),
            _ => None,
        }
    }

    pub fn directory_picker_state(&self) -> Option<&DirectoryPickerState> {
        match self.overlays.dialog.as_ref() {
            Some(DialogState::DirectoryPicker(state)) => Some(state),
            _ => None,
        }
    }

    fn directory_picker_state_mut(&mut self) -> Option<&mut DirectoryPickerState> {
        match self.overlays.dialog.as_mut() {
            Some(DialogState::DirectoryPicker(state)) => Some(state),
            _ => None,
        }
    }

    pub fn prompt_dialog_state(&self) -> Option<&PromptDialogState> {
        match self.overlays.dialog.as_ref() {
            Some(DialogState::Prompt(state)) => Some(state),
            _ => None,
        }
    }

    fn prompt_dialog_state_mut(&mut self) -> Option<&mut PromptDialogState> {
        match self.overlays.dialog.as_mut() {
            Some(DialogState::Prompt(state)) => Some(state),
            _ => None,
        }
    }

    pub fn confirm_dialog_state(&self) -> Option<&ConfirmDialogState> {
        match self.overlays.dialog.as_ref() {
            Some(DialogState::Confirm(state)) => Some(state),
            _ => None,
        }
    }

    pub fn open_prompt_dialog(&mut self, dialog: PromptDialogState) -> bool {
        let changed = self.focus != Focus::Dialog || self.prompt_dialog_state() != Some(&dialog);
        if self.focus != Focus::Dialog {
            self.overlays.previous_focus = Some(self.focus);
        }
        self.overlays.dialog = Some(DialogState::Prompt(dialog));
        self.focus = Focus::Dialog;
        changed
    }

    pub fn open_confirm_dialog(&mut self, dialog: ConfirmDialogState) -> bool {
        let changed = self.focus != Focus::Dialog || self.confirm_dialog_state() != Some(&dialog);
        if self.focus != Focus::Dialog {
            self.overlays.previous_focus = Some(self.focus);
        }
        self.overlays.dialog = Some(DialogState::Confirm(dialog));
        self.focus = Focus::Dialog;
        changed
    }

    pub fn open_directory_picker_dialog(&mut self, dialog: DirectoryPickerState) -> bool {
        let changed = self.focus != Focus::Dialog || self.directory_picker_state() != Some(&dialog);
        if self.focus != Focus::Dialog {
            self.overlays.previous_focus = Some(self.focus);
        }
        self.overlays.dialog = Some(DialogState::DirectoryPicker(dialog));
        self.focus = Focus::Dialog;
        changed
    }

    pub fn open_add_root_directory_picker(&mut self) -> Result<bool> {
        let start_dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        self.open_add_root_directory_picker_at(start_dir)
    }

    pub fn open_add_root_directory_picker_at(&mut self, start_dir: PathBuf) -> Result<bool> {
        let visibility = VisibilitySettings {
            show_hidden: self.config.general.show_hidden,
            respect_gitignore: self.config.general.respect_gitignore,
        };
        let current_dir = fs::canonicalize(&start_dir)?;
        let entries = load_directory_picker_entries(
            &current_dir,
            DirectoryPickerEntryMode::DirectoriesOnly,
            visibility,
        )?;
        Ok(self.open_directory_picker_dialog(DirectoryPickerState {
            intent: DirectoryPickerIntent::AddRoot,
            entry_mode: DirectoryPickerEntryMode::DirectoriesOnly,
            current_dir,
            selected_index: 0,
            entries,
            error_message: None,
            show_hidden: visibility.show_hidden,
            respect_gitignore: visibility.respect_gitignore,
        }))
    }

    pub fn set_directory_picker_selection_by_index(&mut self, index: usize) -> bool {
        let Some(picker) = self.directory_picker_state_mut() else {
            return false;
        };
        if picker.entries.is_empty() {
            return false;
        }
        let next_index = index.min(picker.entries.len() - 1);
        if picker.selected_index == next_index {
            return false;
        }
        picker.selected_index = next_index;
        true
    }

    pub fn move_directory_picker_selection(&mut self, delta: isize) -> bool {
        let Some(picker) = self.directory_picker_state_mut() else {
            return false;
        };
        if picker.entries.is_empty() {
            return false;
        }
        let current = picker.selected_index.min(picker.entries.len() - 1);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.saturating_abs() as usize)
        } else {
            current
                .saturating_add(delta as usize)
                .min(picker.entries.len() - 1)
        };
        if next == current {
            return false;
        }
        picker.selected_index = next;
        true
    }

    pub fn enter_directory_picker_selection(&mut self) -> Result<bool> {
        let Some((path, label, mode, show_hidden, respect_gitignore)) =
            self.directory_picker_state().and_then(|picker| {
                picker.entries.get(picker.selected_index).map(|entry| {
                    (
                        entry.path.clone(),
                        entry.label.clone(),
                        picker.entry_mode,
                        picker.show_hidden,
                        picker.respect_gitignore,
                    )
                })
            })
        else {
            return Ok(false);
        };
        self.reload_directory_picker_state(path, mode, show_hidden, respect_gitignore, Some(label))
    }

    pub fn move_directory_picker_to_parent(&mut self) -> Result<bool> {
        let Some((parent, mode, show_hidden, respect_gitignore)) =
            self.directory_picker_state().and_then(|picker| {
                picker.current_dir.parent().map(|parent| {
                    (
                        parent.to_path_buf(),
                        picker.entry_mode,
                        picker.show_hidden,
                        picker.respect_gitignore,
                    )
                })
            })
        else {
            return Ok(false);
        };
        self.reload_directory_picker_state(parent, mode, show_hidden, respect_gitignore, None)
    }

    pub fn commit_directory_picker_selection(&mut self) -> Result<bool> {
        let Some(target_root) = self
            .directory_picker_state()
            .and_then(|picker| picker.entries.get(picker.selected_index))
            .map(|entry| normalize_root_label_path(&entry.path))
        else {
            return Ok(false);
        };

        let original_pins = self.config.bookmarks.pins.clone();
        let original_selected_root = self.roots.selected_index;
        let pinned = self.pin_bookmark_path(target_root.clone());
        if pinned && let Err(error) = self.save_config() {
            self.config.bookmarks.pins = original_pins;
            self.roots.selected_index = original_selected_root;
            return Err(error);
        }
        let opened = self.open_or_activate_tab(target_root.clone());
        let closed = self.close_directory_picker_dialog();
        Ok(pinned || opened || closed)
    }

    pub fn append_prompt_dialog_char(&mut self, ch: char) -> bool {
        let Some(prompt) = self.prompt_dialog_state_mut() else {
            return false;
        };
        prompt.value.push(ch);
        true
    }

    pub fn backspace_prompt_dialog(&mut self) -> bool {
        let Some(prompt) = self.prompt_dialog_state_mut() else {
            return false;
        };
        prompt.value.pop().is_some()
    }

    pub fn close_directory_picker_dialog(&mut self) -> bool {
        let changed = matches!(self.overlays.dialog, Some(DialogState::DirectoryPicker(_)))
            || self.focus == Focus::Dialog;
        if matches!(self.overlays.dialog, Some(DialogState::DirectoryPicker(_))) {
            self.overlays.dialog = None;
        }
        if self.focus == Focus::Dialog {
            self.focus = self.overlays.previous_focus.take().unwrap_or(Focus::Tree);
        }
        changed
    }

    fn reload_directory_picker_state(
        &mut self,
        next_dir: PathBuf,
        entry_mode: DirectoryPickerEntryMode,
        show_hidden: bool,
        respect_gitignore: bool,
        error_label: Option<String>,
    ) -> Result<bool> {
        let current_dir_before = self
            .directory_picker_state()
            .map(|picker| picker.current_dir.clone());
        let visibility = VisibilitySettings {
            show_hidden,
            respect_gitignore,
        };
        let result = load_directory_picker_entries(&next_dir, entry_mode, visibility);
        let Some(picker) = self.directory_picker_state_mut() else {
            return Ok(false);
        };

        match result {
            Ok(entries) => {
                let canonical_dir = fs::canonicalize(&next_dir)?;
                let changed = current_dir_before.as_ref() != Some(&canonical_dir)
                    || picker.entries != entries
                    || picker.selected_index != 0
                    || picker.error_message.is_some();
                picker.current_dir = canonical_dir;
                picker.entries = entries;
                picker.selected_index = 0;
                picker.error_message = None;
                Ok(changed)
            }
            Err(error) => {
                let message = match error_label {
                    Some(label) => format!("could not open {label}: {error}"),
                    None => format!("could not open {}: {error}", next_dir.display()),
                };
                let changed = picker.error_message.as_deref() != Some(message.as_str());
                picker.error_message = Some(message);
                Ok(changed)
            }
        }
    }

    pub fn close_dialog(&mut self) -> bool {
        let changed = self.overlays.dialog.is_some() || self.focus == Focus::Dialog;
        self.overlays.dialog = None;
        if self.focus == Focus::Dialog {
            self.focus = self.overlays.previous_focus.take().unwrap_or(Focus::Tree);
        }
        changed
    }

    pub fn focus_path_filter(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };

        let changed = self.focus != Focus::PathFilter || !tab.path_filter.active;
        self.focus = Focus::PathFilter;
        tab.path_filter.active = true;
        changed
    }

    pub fn blur_path_filter(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };

        let changed = self.focus != Focus::Tree || tab.path_filter.active;
        self.focus = Focus::Tree;
        tab.path_filter.active = false;
        changed
    }

    pub fn focus_next_panel(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };

        let next_focus = match self.focus {
            Focus::Tree => Focus::Roots,
            Focus::Roots => Focus::Preview,
            Focus::Preview => Focus::Tree,
            Focus::PathFilter => Focus::Tree,
            _ => Focus::Tree,
        };
        let changed = self.focus != next_focus || tab.path_filter.active;
        tab.path_filter.active = false;
        self.focus = next_focus;
        changed
    }

    pub fn append_path_filter_char(&mut self, ch: char) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };

        self.focus = Focus::PathFilter;
        tab.path_filter.active = true;
        tab.append_path_filter_char(ch)
    }

    pub fn backspace_path_filter(&mut self) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };
        tab.backspace_path_filter()
    }

    pub fn open_active_content_search(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let changed = tab.open_content_search() || self.focus != Focus::ContentSearch;
        if self.focus != Focus::ContentSearch {
            self.overlays.previous_focus = Some(self.focus);
            self.focus = Focus::ContentSearch;
        }
        changed
    }

    pub fn selected_directory_root_candidate(&self) -> Option<PathBuf> {
        let tab = self.tabs.get(self.active_tab)?;
        let selected_rel_path = tab.tree.selected_rel_path().unwrap_or_default();
        let node_id = if selected_rel_path.as_os_str().is_empty() {
            tab.tree.root_id
        } else {
            *tab.tree.path_to_id.get(&selected_rel_path)?
        };
        let selected_node = tab.tree.node(node_id)?;
        match selected_node.kind {
            NodeKind::Directory | NodeKind::SymlinkDirectory => {
                Some(tab.root.join(&selected_node.rel_path))
            }
            NodeKind::File | NodeKind::SymlinkFile => None,
        }
    }

    pub fn open_selected_directory_as_root_tab(&mut self, root: PathBuf) -> bool {
        self.open_or_activate_tab(root)
    }

    pub fn active_root_is_bookmarked(&self) -> bool {
        let Some(root) = self.tabs.get(self.active_tab).map(|tab| &tab.root) else {
            return false;
        };
        self.config
            .bookmarks
            .pins
            .iter()
            .any(|pinned| pinned == root)
    }

    pub fn bookmark_target_root(&self) -> Option<PathBuf> {
        let active_root = normalize_root_label_path(&self.tabs.get(self.active_tab)?.root);
        match self.selected_directory_root_candidate() {
            Some(candidate) => {
                let candidate = normalize_root_label_path(&candidate);
                if candidate != active_root {
                    Some(candidate)
                } else {
                    Some(active_root)
                }
            }
            _ => Some(active_root),
        }
    }

    pub fn bookmark_target_is_active_root(&self) -> bool {
        let Some(target) = self.bookmark_target_root() else {
            return false;
        };
        self.tabs
            .get(self.active_tab)
            .is_some_and(|tab| tab.root == target)
    }

    pub fn bookmark_target_is_bookmarked(&self) -> bool {
        let Some(target) = self.bookmark_target_root() else {
            return false;
        };
        self.config
            .bookmarks
            .pins
            .iter()
            .any(|pinned| pinned == &target)
    }

    pub fn toggle_active_root_bookmark(&mut self) -> bool {
        if self.active_root_is_bookmarked() {
            self.unpin_active_root()
        } else {
            self.pin_active_root()
        }
    }

    pub fn toggle_bookmark_target(&mut self) -> bool {
        if self.bookmark_target_is_bookmarked() {
            self.unpin_bookmark_target()
        } else {
            self.pin_bookmark_target()
        }
    }

    pub fn open_unified_command_surface(&mut self) -> bool {
        self.open_command_palette()
    }

    pub fn active_multi_select_mode(&self) -> bool {
        self.tabs
            .get(self.active_tab)
            .is_some_and(|tab| tab.multi_select.active)
    }

    pub fn active_multi_select_count(&self) -> usize {
        self.tabs
            .get(self.active_tab)
            .map_or(0, |tab| tab.multi_select.selected_paths.len())
    }

    pub fn active_multi_selected_paths(&self) -> Vec<PathBuf> {
        self.tabs
            .get(self.active_tab)
            .map_or_else(Vec::new, TabState::multi_selected_paths)
    }

    pub fn active_batchable_selected_rel_path(&self) -> Option<PathBuf> {
        let tab = self.tabs.get(self.active_tab)?;
        let rel_path = tab.tree.selected_rel_path()?;
        (!is_root_rel_path(&rel_path)).then_some(rel_path)
    }

    pub fn active_sendable_rel_paths(&self) -> Vec<PathBuf> {
        let batch_paths = self.active_multi_selected_paths();
        if !batch_paths.is_empty() {
            return batch_paths;
        }
        self.active_batchable_selected_rel_path()
            .into_iter()
            .collect()
    }

    pub fn toggle_active_multi_select_mode(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.toggle_multi_select_mode()
    }

    pub fn exit_active_multi_select_mode(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.exit_multi_select_mode()
    }

    pub fn clear_active_multi_select(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.clear_multi_select()
    }

    pub fn toggle_selected_path_in_active_multi_select(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.toggle_selected_path_in_multi_select()
    }

    pub fn active_git_summary(&self) -> Option<GitRepoSummary> {
        self.tabs.get(self.active_tab)?.git.repo_summary()
    }

    pub fn active_selected_git_path_status(&self) -> Option<GitPathStatus> {
        let tab = self.tabs.get(self.active_tab)?;
        let rel_path = tab.tree.selected_rel_path()?;
        if rel_path.as_os_str().is_empty() {
            return None;
        }

        let node_id = *tab.tree.path_to_id.get(&rel_path)?;
        let node = tab.tree.node(node_id)?;
        if !matches!(node.kind, NodeKind::File | NodeKind::SymlinkFile) {
            return None;
        }

        let _ = tab.git.repo.as_ref()?;
        tab.git.status_map.get(&rel_path).copied()
    }

    pub fn active_selection_supports_unstaged_diff(&self) -> bool {
        self.active_selected_git_path_status()
            .is_some_and(|status| {
                !status.conflicted
                    && !status.ignored
                    && (status.untracked || status.worktree != GitChange::Unmodified)
            })
    }

    pub fn activate_diff_mode_if_available(&mut self) -> bool {
        if self.active_selection_supports_unstaged_diff() {
            let mode_changed = self.set_active_context_mode(ContextMode::Diff);
            let status_changed = self.status.message == DIFF_UNAVAILABLE_MESSAGE;
            if status_changed {
                self.status.message.clear();
            }
            return mode_changed || status_changed;
        }

        let status_changed = self.status.message != DIFF_UNAVAILABLE_MESSAGE;
        self.status.message = DIFF_UNAVAILABLE_MESSAGE.to_string();
        status_changed
    }

    pub fn close_active_content_search(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let changed = tab.close_content_search() || self.focus == Focus::ContentSearch;
        if self.focus == Focus::ContentSearch {
            self.focus = self.overlays.previous_focus.take().unwrap_or(Focus::Tree);
        }
        changed
    }

    pub fn append_active_content_search_char(&mut self, ch: char) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.append_content_search_char(ch)
    }

    pub fn backspace_active_content_search(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.backspace_content_search()
    }

    pub fn set_active_content_search_query(&mut self, query: &str) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.set_content_search_query(query)
    }

    pub fn select_active_content_search_hit(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.select_content_search_hit(index)
    }

    pub fn move_active_content_search_selection(&mut self, delta: isize) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.move_content_search_selection(delta)
    }

    pub fn open_command_palette(&mut self) -> bool {
        let entries = crate::actions::catalog::command_palette_entries(self, "");
        let changed = !self.overlays.command_palette.active || self.focus != Focus::CommandPalette;
        self.overlays.previous_focus = Some(self.focus);
        self.overlays.command_palette.query.clear();
        self.overlays.command_palette.selected_index = 0;
        self.overlays.command_palette.entries = entries;
        self.overlays.command_palette.active = true;
        self.focus = Focus::CommandPalette;
        changed
    }

    pub fn close_command_palette(&mut self) -> bool {
        let changed = self.overlays.command_palette.active || self.focus == Focus::CommandPalette;
        self.overlays.command_palette.active = false;
        self.overlays.command_palette.query.clear();
        self.overlays.command_palette.selected_index = 0;
        self.overlays.command_palette.entries.clear();
        if self.focus == Focus::CommandPalette {
            self.focus = self.overlays.previous_focus.take().unwrap_or(Focus::Tree);
        }
        changed
    }

    pub fn append_command_palette_char(&mut self, ch: char) -> bool {
        let mut next_query = self.overlays.command_palette.query.clone();
        next_query.push(ch);
        self.set_command_palette_query(&next_query)
    }

    pub fn backspace_command_palette(&mut self) -> bool {
        if self.overlays.command_palette.query.is_empty() {
            return false;
        }
        let mut next_query = self.overlays.command_palette.query.clone();
        next_query.pop();
        self.set_command_palette_query(&next_query)
    }

    pub fn set_command_palette_query(&mut self, query: &str) -> bool {
        let next_entries = crate::actions::catalog::command_palette_entries(self, query);
        let next_selected = clamp_overlay_selection(
            self.overlays.command_palette.selected_index,
            next_entries.len(),
        );
        let changed = self.overlays.command_palette.query != query
            || self.overlays.command_palette.entries != next_entries
            || self.overlays.command_palette.selected_index != next_selected;
        self.overlays.command_palette.query = query.to_string();
        self.overlays.command_palette.entries = next_entries;
        self.overlays.command_palette.selected_index = next_selected;
        changed
    }

    pub fn move_command_palette_selection(&mut self, delta: isize) -> bool {
        move_overlay_selection(
            &mut self.overlays.command_palette.selected_index,
            self.overlays.command_palette.entries.len(),
            delta,
        )
    }

    pub fn commit_command_palette_action(&mut self) -> Option<crate::action::Action> {
        let action = self
            .overlays
            .command_palette
            .entries
            .get(self.overlays.command_palette.selected_index)
            .filter(|entry| entry.enabled)
            .map(|entry| entry.action.clone())?;
        let _ = self.close_command_palette();
        Some(action)
    }

    pub fn activate_selected_content_search_hit(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let Some(hit) = tab.selected_content_search_hit().cloned() else {
            return false;
        };

        let rel_path = PathBuf::from(&hit.path);
        let revealed = tab.reveal_rel_path_in_tree(&rel_path);
        if !revealed && tab.tree.selected_rel_path().as_deref() != Some(rel_path.as_path()) {
            return false;
        }
        let line_index = hit.line.saturating_sub(1);
        let mode_changed = tab.mode != ContextMode::Preview;
        tab.mode = ContextMode::Preview;
        tab.preview.source.rel_path = None;
        tab.preview.source.context_mode = ContextMode::Preview;
        tab.preview.render_cache = None;
        tab.preview.scroll_row = line_index;
        tab.preview.editor_line_hint = Some(hit.line.max(1));
        tab.preview.selected_line_start = Some(line_index);
        tab.preview.selected_line_end = Some(line_index);
        tab.content_search.active = false;
        self.overlays.previous_focus = None;
        let focus_changed = self.focus != Focus::Preview;
        self.focus = Focus::Preview;

        revealed || mode_changed || focus_changed
    }

    pub fn poll_active_tab_path_index(&mut self) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };
        tab.poll_path_index()
    }

    pub fn submit_active_content_search(&mut self) -> Result<bool> {
        let active_tab = self.active_tab;
        let queued = {
            let Some(tab) = self.tabs.get_mut(active_tab) else {
                return Ok(false);
            };
            if tab.content_search.query.trim().is_empty() {
                return Ok(false);
            }
            tab.resubmit_content_search_with_current_snapshot()
        };

        let mode_changed = self.set_active_context_mode(ContextMode::SearchResults);
        Ok(queued || mode_changed)
    }

    pub fn poll_active_tab_content_search(&mut self) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };

        let (latest_response, disconnected) = {
            let Some(worker) = tab.content_search.runtime.worker.as_ref() else {
                return Ok(false);
            };
            let mut latest_response = None;
            let mut disconnected = false;
            loop {
                match worker.try_recv() {
                    Ok(response) => latest_response = Some(response),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            (latest_response, disconnected)
        };

        let mut changed = false;
        if let Some(response) = latest_response {
            changed |= tab.apply_content_search_response(response);
            if matches!(tab.content_search.status, ContentSearchStatus::Ready) {
                tab.content_search.runtime.worker = None;
            }
        }
        if disconnected {
            tab.content_search.runtime.worker = None;
            if !matches!(tab.content_search.status, ContentSearchStatus::Ready) {
                tab.content_search.status = ContentSearchStatus::Error;
                tab.content_search.status_message =
                    Some("content search worker disconnected".to_string());
                changed = true;
            }
        }

        Ok(changed)
    }

    pub fn poll_active_tab_mermaid_render(&mut self) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };
        Ok(tab.poll_mermaid_render())
    }

    pub fn poll_active_tab_image_render(&mut self) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };
        Ok(tab.poll_image_render())
    }

    pub fn refresh_active_preview(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.refresh_preview(&self.config.preview)
    }

    pub fn refresh_active_git_state(&mut self) -> Result<bool> {
        let backend = LibgitBackend;
        self.refresh_active_git_state_with_backend(&backend)
    }

    pub fn refresh_active_git_state_with_backend<B: GitBackend>(
        &mut self,
        backend: &B,
    ) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };
        tab.refresh_git_state(backend)
    }

    pub fn sync_active_git_badges(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.sync_git_badges()
    }

    pub fn set_active_context_mode(&mut self, mode: ContextMode) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        if tab.mode == mode {
            return false;
        }

        tab.mode = mode;
        tab.preview.source.rel_path = None;
        tab.preview.source.context_mode = mode;
        tab.preview.render_cache = None;
        tab.preview.scroll_row = 0;
        tab.preview.editor_line_hint = None;
        true
    }

    pub fn invalidate_active_preview(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let changed = tab.preview.source.rel_path.is_some() || tab.preview.render_cache.is_some();
        tab.preview.source.rel_path = None;
        tab.preview.render_cache = None;
        tab.preview.scroll_row = 0;
        tab.preview.editor_line_hint = None;
        changed
    }

    pub fn refresh_active_tab_after_file_op(
        &mut self,
        reveal_rel_path: Option<&Path>,
    ) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };

        let changed = tab.refresh_tree_after_file_op(reveal_rel_path)?;
        let _ = tab.invalidate_preview();
        if tab.content_search.active
            || !tab.content_search.payload.hits.is_empty()
            || !tab.content_search.query.is_empty()
        {
            tab.content_search.generation.0 = tab.content_search.generation.0.saturating_add(1);
            tab.content_search.status = ContentSearchStatus::Idle;
            tab.content_search.status_message = None;
            tab.content_search.payload = SearchPayload {
                query: tab.content_search.query.clone(),
                hits: Vec::new(),
            };
            tab.content_search.selected_hit_index = None;
            tab.content_search.runtime.worker = None;
        }
        tab.git.needs_refresh = true;
        Ok(changed)
    }

    pub fn apply_watcher_refresh_plan(&mut self, plan: &RefreshPlan) -> Result<bool> {
        let normalized_plan_root = normalize_root_label_path(&plan.root);
        let mut recovery_message = None;
        let mut diff_fallback_message = None;
        let Some(tab_index) = self
            .tabs
            .iter()
            .position(|tab| normalize_root_label_path(&tab.root) == normalized_plan_root)
        else {
            return Ok(false);
        };

        if self
            .tabs
            .get(tab_index)
            .is_some_and(|tab| !tab.root.exists())
        {
            if let Some(message) = self.recover_missing_root_tab(tab_index) {
                self.status.severity = StatusSeverity::Warning;
                self.status.message = message;
                return Ok(true);
            }
            return Ok(false);
        }

        let tab = &mut self.tabs[tab_index];
        let selected_rel_path_before_refresh = tab.tree.selected_rel_path();
        let preview_rel_path_before_refresh = tab.preview.source.rel_path.clone();
        let preview_context_before_refresh = tab.preview.source.context_mode;
        let (mut changed, selection_recovery) = tab.refresh_tree_after_watcher_refresh()?;
        changed |= tab.invalidate_content_search_results();
        if plan.git_dirty {
            tab.git.needs_refresh = true;
        }

        let selection_changed = selection_recovery.is_some()
            || selected_rel_path_before_refresh != tab.tree.selected_rel_path();
        let preview_target_touched = preview_rel_path_before_refresh
            .as_deref()
            .is_some_and(|rel_path| watcher_plan_touches_rel_path(plan, rel_path));
        let diff_refresh_needed = preview_context_before_refresh == ContextMode::Diff
            && (selection_changed || preview_target_touched || plan.git_dirty);

        if diff_refresh_needed {
            if tab.selected_path_has_unstaged_diff() {
                changed |= tab.invalidate_preview();
            } else {
                if tab.mode != ContextMode::Preview {
                    tab.mode = ContextMode::Preview;
                    changed = true;
                }
                changed |= tab.invalidate_preview();
                if tab_index == self.active_tab {
                    diff_fallback_message = Some(DIFF_UNAVAILABLE_MESSAGE.to_string());
                }
            }
        } else if preview_context_before_refresh == ContextMode::Preview
            && (selection_changed || preview_target_touched)
        {
            changed |= tab.invalidate_preview();
        }

        if tab_index == self.active_tab
            && let Some((missing_rel_path, _)) = selection_recovery
            && let Some(actual_selected_rel_path) = tab.tree.selected_rel_path()
        {
            recovery_message = Some(format!(
                "watcher refresh recovered selection: {} -> {}",
                missing_rel_path.display(),
                actual_selected_rel_path.display()
            ));
        }

        if let Some(message) = recovery_message {
            self.status.severity = StatusSeverity::Warning;
            self.status.message = message;
        }
        if let Some(message) = diff_fallback_message {
            self.status.severity = StatusSeverity::Warning;
            self.status.message = message;
        }

        Ok(changed)
    }

    fn recover_missing_root_tab(&mut self, tab_index: usize) -> Option<String> {
        let missing_root = self.tabs.get(tab_index)?.root.clone();

        if self.tabs.len() > 1 {
            self.tabs.remove(tab_index);
            if self.active_tab > tab_index {
                self.active_tab = self.active_tab.saturating_sub(1);
            }
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
            let next_root = self.tabs.get(self.active_tab)?.root.clone();
            let _ = self.select_root_path(&next_root);
            return Some(format!(
                "watcher refresh closed missing root: {} -> {}",
                missing_root.display(),
                next_root.display()
            ));
        }

        let visibility = self.tabs.get(tab_index)?.tree.visibility_settings();
        let split_ratio = self.tabs.get(tab_index)?.split_ratio;
        let recovery_root = nearest_surviving_root_path(&missing_root);
        let mut replacement = TabState::new_with_visibility(recovery_root.clone(), visibility);
        replacement.split_ratio = split_ratio;
        self.tabs[tab_index] = replacement;
        self.active_tab = tab_index.min(self.tabs.len().saturating_sub(1));
        let _ = self.select_root_path(&recovery_root);
        Some(format!(
            "watcher refresh recovered missing root: {} -> {}",
            missing_root.display(),
            recovery_root.display()
        ))
    }

    pub fn toggle_show_hidden(&mut self) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };
        let changed = tab.toggle_show_hidden()?;
        if changed {
            self.config.general.show_hidden = tab.tree.show_hidden;
        }
        Ok(changed)
    }

    pub fn toggle_respect_gitignore(&mut self) -> Result<bool> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Ok(false);
        };
        let changed = tab.toggle_respect_gitignore()?;
        if changed {
            self.config.general.respect_gitignore = tab.tree.respect_gitignore;
        }
        Ok(changed)
    }

    pub fn save_config(&self) -> Result<()> {
        let Some(path) = &self.config_path else {
            return Err(std::io::Error::other("config path is unavailable").into());
        };

        self.config.save_to_path(path)
    }

    pub fn bookmark_paths(&self) -> &[PathBuf] {
        &self.config.bookmarks.pins
    }

    pub fn root_navigator_entries(&self) -> Vec<RootNavigatorEntry> {
        let label_map = self.root_display_labels();
        let mut entries = Vec::new();

        for path in self.bookmark_paths() {
            let normalized = normalize_root_label_path(path);
            let label = label_map
                .get(&normalized)
                .cloned()
                .unwrap_or_else(|| RootDisplayLabel {
                    primary: root_label(path),
                    disambiguator: None,
                });
            entries.push(RootNavigatorEntry {
                path: path.clone(),
                section: RootNavigatorSection::Pinned,
                label: label.primary,
                disambiguator: label.disambiguator,
                pinned: true,
                open: self.bookmark_is_open(path),
                active: self.bookmark_is_active(path),
            });
        }

        for tab in &self.tabs {
            let tab_root = normalize_root_label_path(&tab.root);
            if self
                .bookmark_paths()
                .iter()
                .any(|path| normalize_root_label_path(path) == tab_root)
            {
                continue;
            }

            let label = label_map
                .get(&tab_root)
                .cloned()
                .unwrap_or_else(|| RootDisplayLabel {
                    primary: root_label(&tab.root),
                    disambiguator: None,
                });
            entries.push(RootNavigatorEntry {
                path: tab.root.clone(),
                section: RootNavigatorSection::Open,
                label: label.primary,
                disambiguator: label.disambiguator,
                pinned: false,
                open: true,
                active: self
                    .tabs
                    .get(self.active_tab)
                    .is_some_and(|active_tab| active_tab.root == tab.root),
            });
        }

        entries
    }

    pub fn root_navigator_counts(&self) -> (usize, usize) {
        self.root_navigator_entries()
            .into_iter()
            .fold((0usize, 0usize), |(pinned, open), entry| {
                match entry.section {
                    RootNavigatorSection::Pinned => (pinned + 1, open),
                    RootNavigatorSection::Open => (pinned, open + 1),
                }
            })
    }

    pub fn root_navigator_panel_height(&self) -> u16 {
        let (pinned, open) = self.root_navigator_counts();
        let mut text_lines = 0usize;
        if pinned > 0 {
            text_lines += 1 + pinned;
        }
        if open > 0 {
            text_lines += 1 + open;
        }
        if text_lines == 0 {
            text_lines = 1;
        }
        (text_lines as u16 + 2).clamp(6, 10)
    }

    pub fn selected_root_index(&self) -> Option<usize> {
        let len = self.root_navigator_entries().len();
        if len == 0 {
            None
        } else {
            Some(self.roots.selected_index.min(len - 1))
        }
    }

    pub fn selected_root_path(&self) -> Option<PathBuf> {
        let index = self.selected_root_index()?;
        self.root_navigator_entries()
            .get(index)
            .map(|entry| entry.path.clone())
    }

    pub fn move_root_selection(&mut self, delta: isize) -> bool {
        let len = self.root_navigator_entries().len();
        if len == 0 {
            return false;
        }

        let current = self.selected_root_index().unwrap_or(0);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.saturating_abs() as usize)
        } else {
            current.saturating_add(delta as usize).min(len - 1)
        };
        if next == current {
            return false;
        }

        self.roots.selected_index = next;
        true
    }

    pub fn select_root_path(&mut self, path: &Path) -> bool {
        let Some(index) = self
            .root_navigator_entries()
            .iter()
            .position(|entry| entry.path == path)
        else {
            return false;
        };
        if self.roots.selected_index == index {
            return false;
        }
        self.roots.selected_index = index;
        true
    }

    pub fn pin_active_root(&mut self) -> bool {
        let Some(root) = self.tabs.get(self.active_tab).map(|tab| tab.root.clone()) else {
            return false;
        };
        self.pin_bookmark_path(root)
    }

    pub fn unpin_active_root(&mut self) -> bool {
        let Some(root) = self.tabs.get(self.active_tab).map(|tab| tab.root.clone()) else {
            return false;
        };
        self.unpin_bookmark_path(&root)
    }

    pub fn pin_bookmark_target(&mut self) -> bool {
        let Some(root) = self.bookmark_target_root() else {
            return false;
        };
        self.pin_bookmark_path(root)
    }

    pub fn unpin_bookmark_target(&mut self) -> bool {
        let Some(root) = self.bookmark_target_root() else {
            return false;
        };
        self.unpin_bookmark_path(&root)
    }

    pub fn bookmark_is_open(&self, path: &Path) -> bool {
        let target = normalize_root_label_path(path);
        self.tabs
            .iter()
            .any(|tab| normalize_root_label_path(&tab.root) == target)
    }

    pub fn bookmark_is_active(&self, path: &Path) -> bool {
        let target = normalize_root_label_path(path);
        self.tabs
            .get(self.active_tab)
            .is_some_and(|tab| normalize_root_label_path(&tab.root) == target)
    }

    pub fn activate_selected_root(&mut self) -> bool {
        let Some(path) = self.selected_root_path() else {
            return false;
        };
        self.open_or_activate_tab(path)
    }

    pub fn open_or_activate_tab(&mut self, root: PathBuf) -> bool {
        let root = normalize_root_label_path(&root);
        if let Some(index) = self.tabs.iter().position(|tab| tab.root == root) {
            if self.active_tab == index {
                return false;
            }
            self.active_tab = index;
            let _ = self.select_root_path(&root);
            return true;
        }

        let visibility = self
            .tabs
            .get(self.active_tab)
            .map(|tab| tab.tree.visibility_settings())
            .unwrap_or(VisibilitySettings {
                show_hidden: self.config.general.show_hidden,
                respect_gitignore: self.config.general.respect_gitignore,
            });
        let split_ratio = self
            .tabs
            .get(self.active_tab)
            .map(|tab| tab.split_ratio)
            .unwrap_or(self.config.layout.split_ratio);
        let mut tab = TabState::new_with_visibility(root, visibility);
        tab.split_ratio = split_ratio;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len().saturating_sub(1);
        let active_root = self.tabs[self.active_tab].root.clone();
        let _ = self.select_root_path(&active_root);
        true
    }

    pub fn move_active_tab(&mut self, delta: isize) -> bool {
        let len = self.tabs.len();
        if len <= 1 {
            return false;
        }

        let current = self.active_tab;
        let next = if delta.is_negative() {
            current.saturating_sub(delta.saturating_abs() as usize)
        } else {
            current.saturating_add(delta as usize).min(len - 1)
        };
        if next == current {
            return false;
        }

        self.active_tab = next;
        true
    }

    pub fn close_active_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 || self.active_tab >= self.tabs.len() {
            return false;
        }

        self.tabs.remove(self.active_tab);
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        if let Some(active_root) = self.tabs.get(self.active_tab).map(|tab| tab.root.clone()) {
            let _ = self.select_root_path(&active_root);
        }
        true
    }

    pub fn tab_label(&self, index: usize) -> Option<String> {
        self.tab_display_labels()
            .get(index)
            .map(RootDisplayLabel::text)
    }

    pub fn bookmark_label(&self, index: usize) -> Option<String> {
        self.bookmark_display_labels()
            .get(index)
            .map(RootDisplayLabel::text)
    }

    pub fn open_target_picker(
        &mut self,
        role: crate::bridge::protocol::TargetRole,
        sessions: Vec<SessionSummary>,
    ) -> bool {
        let has_current_pane = matches!(role, crate::bridge::protocol::TargetRole::Editor);
        if sessions.is_empty() && !has_current_pane {
            return false;
        }

        if self.focus != Focus::Dialog {
            self.overlays.previous_focus = Some(self.focus);
        }

        self.bridge.session_summaries = sessions;
        self.overlays.dialog = Some(DialogState::TargetPicker(TargetPickerState {
            role,
            selection: match role {
                crate::bridge::protocol::TargetRole::Editor => TargetPickerSelection::CurrentPane,
                crate::bridge::protocol::TargetRole::Ai => self
                    .bridge
                    .ai_target_session_id
                    .clone()
                    .or_else(|| {
                        self.bridge
                            .session_summaries
                            .first()
                            .map(|session| session.session_id.clone())
                    })
                    .map(TargetPickerSelection::SessionId)
                    .unwrap_or(TargetPickerSelection::CurrentPane),
                crate::bridge::protocol::TargetRole::Grove => self
                    .bridge
                    .session_summaries
                    .first()
                    .map(|session| TargetPickerSelection::SessionId(session.session_id.clone()))
                    .unwrap_or(TargetPickerSelection::CurrentPane),
            },
        }));
        self.focus = Focus::Dialog;
        true
    }

    pub(crate) fn tab_display_labels(&self) -> Vec<RootDisplayLabel> {
        let label_map = self.root_display_labels();
        self.tabs
            .iter()
            .map(|tab| {
                label_map
                    .get(&normalize_root_label_path(&tab.root))
                    .cloned()
                    .unwrap_or_else(|| RootDisplayLabel {
                        primary: root_label(&tab.root),
                        disambiguator: None,
                    })
            })
            .collect()
    }

    pub(crate) fn bookmark_display_labels(&self) -> Vec<RootDisplayLabel> {
        let label_map = self.root_display_labels();
        self.bookmark_paths()
            .iter()
            .map(|path| {
                label_map
                    .get(&normalize_root_label_path(path))
                    .cloned()
                    .unwrap_or_else(|| RootDisplayLabel {
                        primary: root_label(path),
                        disambiguator: None,
                    })
            })
            .collect()
    }

    fn pin_bookmark_path(&mut self, root: PathBuf) -> bool {
        let root = normalize_root_label_path(&root);
        if self
            .config
            .bookmarks
            .pins
            .iter()
            .any(|pinned| pinned == &root)
        {
            return false;
        }

        self.config.bookmarks.pins.push(root.clone());
        let _ = self.select_root_path(&root);
        true
    }

    fn unpin_bookmark_path(&mut self, root: &Path) -> bool {
        let root = normalize_root_label_path(root);
        let Some(index) = self
            .config
            .bookmarks
            .pins
            .iter()
            .position(|pinned| pinned == &root)
        else {
            return false;
        };

        self.config.bookmarks.pins.remove(index);
        if !self.select_root_path(&root)
            && let Some(active_root) = self.tabs.get(self.active_tab).map(|tab| tab.root.clone())
        {
            let _ = self.select_root_path(&active_root);
        }
        true
    }

    fn root_display_labels(&self) -> HashMap<PathBuf, RootDisplayLabel> {
        let mut roots = Vec::new();
        for tab in &self.tabs {
            push_unique_root(&mut roots, &normalize_root_label_path(&tab.root));
        }
        for bookmark in &self.config.bookmarks.pins {
            push_unique_root(&mut roots, &normalize_root_label_path(bookmark));
        }
        roots
            .into_iter()
            .map(|display_path| {
                let primary = root_label(&display_path);
                let colliding_roots = self
                    .tabs
                    .iter()
                    .map(|tab| normalize_root_label_path(&tab.root))
                    .chain(
                        self.config
                            .bookmarks
                            .pins
                            .iter()
                            .map(|path| normalize_root_label_path(path)),
                    )
                    .filter(|other| root_label(other) == primary)
                    .fold(Vec::new(), |mut acc, other| {
                        push_unique_root(&mut acc, &other);
                        acc
                    });

                (
                    display_path.clone(),
                    RootDisplayLabel {
                        primary,
                        disambiguator: root_label_disambiguator(&display_path, &colliding_roots),
                    },
                )
            })
            .collect()
    }

    pub fn close_target_picker(&mut self) {
        let _ = self.close_dialog();
    }

    fn target_picker_has_current_pane_option(&self) -> bool {
        self.target_picker_state()
            .is_some_and(|picker| picker.role == crate::bridge::protocol::TargetRole::Editor)
    }

    fn target_picker_option_count(&self) -> usize {
        self.bridge.session_summaries.len()
            + usize::from(self.target_picker_has_current_pane_option())
    }

    fn target_picker_selection_for_index(&self, index: usize) -> Option<TargetPickerSelection> {
        if self.target_picker_has_current_pane_option() {
            if index == 0 {
                return Some(TargetPickerSelection::CurrentPane);
            }
            return self
                .bridge
                .session_summaries
                .get(index.saturating_sub(1))
                .map(|session| TargetPickerSelection::SessionId(session.session_id.clone()));
        }

        self.bridge
            .session_summaries
            .get(index)
            .map(|session| TargetPickerSelection::SessionId(session.session_id.clone()))
    }

    pub fn target_picker_selected_index(&self) -> Option<usize> {
        let picker = self.target_picker_state()?;
        match &picker.selection {
            TargetPickerSelection::CurrentPane if self.target_picker_has_current_pane_option() => {
                Some(0)
            }
            TargetPickerSelection::SessionId(selected_session_id) => self
                .bridge
                .session_summaries
                .iter()
                .position(|session| session.session_id == *selected_session_id)
                .map(|index| index + usize::from(self.target_picker_has_current_pane_option())),
            TargetPickerSelection::CurrentPane => None,
        }
        .or({
            if self.target_picker_option_count() == 0 {
                None
            } else {
                Some(0)
            }
        })
    }

    pub fn target_picker_selected_session(&self) -> Option<&SessionSummary> {
        let picker = self.target_picker_state()?;
        match &picker.selection {
            TargetPickerSelection::CurrentPane => None,
            TargetPickerSelection::SessionId(selected_session_id) => self
                .bridge
                .session_summaries
                .iter()
                .find(|session| session.session_id == *selected_session_id),
        }
    }

    pub fn target_picker_selected_label(&self) -> Option<String> {
        let picker = self.target_picker_state()?;
        match &picker.selection {
            TargetPickerSelection::CurrentPane => Some("Current pane".to_string()),
            TargetPickerSelection::SessionId(selected_session_id) => self
                .bridge
                .session_summaries
                .iter()
                .find(|session| session.session_id == *selected_session_id)
                .map(|session| session.title.clone()),
        }
        .or_else(|| {
            if picker.role == crate::bridge::protocol::TargetRole::Editor {
                Some("Current pane".to_string())
            } else {
                None
            }
        })
    }

    pub fn move_target_picker_selection(&mut self, delta: isize) -> bool {
        let len = self.target_picker_option_count();
        if len == 0 {
            return false;
        }

        let current_index = self.target_picker_selected_index().unwrap_or(0);
        let next_index = if delta.is_negative() {
            current_index.saturating_sub(delta.saturating_abs() as usize)
        } else {
            current_index.saturating_add(delta as usize).min(len - 1)
        };
        let Some(next_selection) = self.target_picker_selection_for_index(next_index) else {
            return false;
        };

        let Some(picker) = self.target_picker_state_mut() else {
            return false;
        };
        let changed = picker.selection.ne(&next_selection);
        picker.selection = next_selection;
        changed
    }

    pub fn set_target_picker_selection_by_index(&mut self, index: usize) -> bool {
        let Some(selection) = self.target_picker_selection_for_index(index) else {
            return false;
        };
        let Some(picker) = self.target_picker_state_mut() else {
            return false;
        };

        let changed = picker.selection != selection;
        picker.selection = selection;
        changed
    }

    pub fn bridge_target_label(&self, role: crate::bridge::protocol::TargetRole) -> String {
        let session_id = match role {
            crate::bridge::protocol::TargetRole::Ai => self.bridge.ai_target_session_id.as_ref(),
            crate::bridge::protocol::TargetRole::Editor => {
                let Some(session_id) = self.bridge.editor_target_session_id.as_ref() else {
                    return "current pane".to_string();
                };
                Some(session_id)
            }
            crate::bridge::protocol::TargetRole::Grove => self.bridge.instance_id.as_ref(),
        };
        let Some(session_id) = session_id else {
            return "unset".to_string();
        };

        self.bridge
            .session_summaries
            .iter()
            .find(|session| session.session_id == *session_id)
            .map(|session| session.title.clone())
            .unwrap_or_else(|| session_id.clone())
    }

    pub fn target_picker_current_label(&self, role: crate::bridge::protocol::TargetRole) -> String {
        match role {
            crate::bridge::protocol::TargetRole::Editor => self.bridge_target_label(role),
            _ => self.bridge_target_label(role),
        }
    }

    pub fn clamp_active_preview_scroll(
        &mut self,
        preview_width: u16,
        viewport_height: usize,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let _ = tab
            .preview
            .clamp_preview_selection(crate::preview::render::line_count_from_cache(
                tab.preview.render_cache.as_ref(),
            ));
        tab.preview.clamp_scroll(preview_width, viewport_height)
    }

    pub fn refresh_active_preview_render_cache(&mut self, preview_width: u16) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let changed = tab.preview.refresh_render_cache(preview_width);
        let _ = tab
            .preview
            .clamp_preview_selection(crate::preview::render::line_count_from_cache(
                tab.preview.render_cache.as_ref(),
            ));
        changed
    }

    pub fn scroll_active_preview_up(&mut self, preview_width: u16, viewport_height: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let changed = tab.preview.scroll_by(-1, preview_width, viewport_height);
        if changed {
            let line_count =
                crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
            let _ = tab
                .preview
                .set_cursor_line(tab.preview.scroll_row, line_count);
        }
        changed
    }

    pub fn scroll_active_preview_down(
        &mut self,
        preview_width: u16,
        viewport_height: usize,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let changed = tab.preview.scroll_by(1, preview_width, viewport_height);
        if changed {
            let line_count =
                crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
            let _ = tab
                .preview
                .set_cursor_line(tab.preview.scroll_row, line_count);
        }
        changed
    }

    pub fn scroll_active_preview_page_up(
        &mut self,
        preview_width: u16,
        viewport_height: usize,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let changed = tab.preview.scroll_by(
            -(viewport_height.max(1) as isize),
            preview_width,
            viewport_height,
        );
        if changed {
            let line_count =
                crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
            let _ = tab
                .preview
                .set_cursor_line(tab.preview.scroll_row, line_count);
        }
        changed
    }

    pub fn scroll_active_preview_page_down(
        &mut self,
        preview_width: u16,
        viewport_height: usize,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let changed = tab.preview.scroll_by(
            viewport_height.max(1) as isize,
            preview_width,
            viewport_height,
        );
        if changed {
            let line_count =
                crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
            let _ = tab
                .preview
                .set_cursor_line(tab.preview.scroll_row, line_count);
        }
        changed
    }

    pub fn scroll_active_preview_home(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let changed = tab.preview.scroll_to_top();
        if changed {
            let line_count =
                crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
            let _ = tab.preview.set_cursor_line(0, line_count);
        }
        changed
    }

    pub fn scroll_active_preview_end(
        &mut self,
        preview_width: u16,
        viewport_height: usize,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let changed = tab.preview.scroll_to_bottom(preview_width, viewport_height);
        if changed {
            let line_count =
                crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
            let _ = tab
                .preview
                .set_cursor_line(tab.preview.scroll_row, line_count);
        }
        changed
    }

    pub fn set_active_preview_cursor_line(&mut self, preview_width: u16, line: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let line_count =
            crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
        tab.preview.set_cursor_line(line, line_count)
    }

    pub fn extend_active_preview_selection(&mut self, preview_width: u16, delta: isize) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let _ = tab.preview.refresh_render_cache(preview_width);
        let line_count =
            crate::preview::render::line_count_from_cache(tab.preview.render_cache.as_ref());
        tab.preview.extend_selection_by(delta, line_count)
    }

    pub fn clear_active_preview_selection(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        tab.preview.clear_preview_selection()
    }
}

#[derive(Debug)]
pub struct TabState {
    pub root: PathBuf,
    pub mode: ContextMode,
    pub tree: TreeState,
    pub multi_select: MultiSelectState,
    pub preview: PreviewState,
    pub image_runtime: ImagePreviewRuntime,
    pub mermaid_runtime: MermaidPreviewRuntime,
    pub path_filter: PathFilterState,
    pub path_index: PathIndexState,
    pub content_search: ContentSearchState,
    pub git: GitTabState,
    pub split_ratio: f32,
}

impl Default for TabState {
    fn default() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(root)
    }
}

impl TabState {
    pub fn new(root: PathBuf) -> Self {
        Self::new_with_visibility(root, VisibilitySettings::default())
    }

    pub fn new_with_visibility(root: PathBuf, visibility: VisibilitySettings) -> Self {
        let tree = crate::tree::loader::load_root_shallow_with_visibility(&root, visibility)
            .unwrap_or_else(|_| fallback_tree(root.clone(), visibility));
        let root_abs = tree.root_abs.clone();

        Self {
            root: root_abs.clone(),
            mode: ContextMode::Preview,
            tree,
            multi_select: MultiSelectState::default(),
            preview: PreviewState::default(),
            image_runtime: ImagePreviewRuntime::default(),
            mermaid_runtime: MermaidPreviewRuntime::default(),
            path_filter: PathFilterState::default(),
            path_index: PathIndexState::default(),
            content_search: ContentSearchState::default(),
            git: GitTabState::default(),
            split_ratio: 0.40,
        }
    }

    pub fn append_path_filter_char(&mut self, ch: char) -> Result<bool> {
        let mut next_query = self.path_filter.query.clone();
        next_query.push(ch);
        self.set_path_filter_query(&next_query)
    }

    pub fn multi_selected_paths(&self) -> Vec<PathBuf> {
        self.multi_select.selected_paths.iter().cloned().collect()
    }

    pub fn toggle_multi_select_mode(&mut self) -> bool {
        self.multi_select.active = !self.multi_select.active;
        true
    }

    pub fn exit_multi_select_mode(&mut self) -> bool {
        if !self.multi_select.active {
            return false;
        }
        self.multi_select.active = false;
        true
    }

    pub fn clear_multi_select(&mut self) -> bool {
        if self.multi_select.selected_paths.is_empty() {
            return false;
        }
        self.multi_select.selected_paths.clear();
        true
    }

    fn reconcile_multi_select_paths(&mut self) -> bool {
        let previous_len = self.multi_select.selected_paths.len();
        let root = self.root.clone();
        self.multi_select.selected_paths.retain(|rel_path| {
            !is_root_rel_path(rel_path) && rel_path_exists_under_root(&root, rel_path)
        });
        self.multi_select.selected_paths.len() != previous_len
    }

    pub fn toggle_selected_path_in_multi_select(&mut self) -> bool {
        if !self.multi_select.active {
            return false;
        }
        let Some(rel_path) = self.tree.selected_rel_path() else {
            return false;
        };
        if is_root_rel_path(&rel_path) {
            return false;
        }
        if !self.multi_select.selected_paths.insert(rel_path.clone()) {
            self.multi_select.selected_paths.remove(&rel_path);
        }
        true
    }

    pub fn backspace_path_filter(&mut self) -> Result<bool> {
        if self.path_filter.query.is_empty() {
            return Ok(false);
        }

        let mut next_query = self.path_filter.query.clone();
        next_query.pop();
        self.set_path_filter_query(&next_query)
    }

    pub fn set_path_filter_query(&mut self, query: &str) -> Result<bool> {
        if self.path_filter.query == query {
            return Ok(false);
        }

        let was_empty = self.path_filter.query.is_empty();
        let will_be_empty = query.is_empty();

        if was_empty && !will_be_empty {
            self.tree.capture_pre_filter_state();
        }

        self.path_filter.query = query.to_string();

        if will_be_empty {
            self.tree.restore_pre_filter_state();
            return Ok(true);
        }

        let _ = self.start_path_index_if_needed();
        self.apply_active_path_filter();
        Ok(true)
    }

    pub fn poll_path_index(&mut self) -> Result<bool> {
        let started_at = Instant::now();
        let Some(receiver) = self.path_index.receiver.take() else {
            return Ok(false);
        };

        let mut changed = false;
        let mut batch_count = 0usize;
        let mut entry_count = 0usize;
        let mut receiver_retained = false;
        let terminal_state = loop {
            match receiver.try_recv() {
                Ok(PathIndexEvent::Batch(entries)) => {
                    batch_count = batch_count.saturating_add(1);
                    entry_count = entry_count.saturating_add(entries.len());
                    changed |= self.ingest_path_index_batch(entries);
                    if batch_count >= MAX_PATH_INDEX_BATCHES_PER_POLL {
                        self.path_index.receiver = Some(receiver);
                        receiver_retained = true;
                        break "budget_exhausted";
                    }
                }
                Ok(PathIndexEvent::Complete) => {
                    self.path_index.status = PathIndexStatus::Ready;
                    changed = true;
                    break "complete";
                }
                Ok(PathIndexEvent::Error(message)) => {
                    self.path_index.status = PathIndexStatus::Error(message);
                    changed = true;
                    break "error";
                }
                Err(TryRecvError::Empty) => {
                    self.path_index.receiver = Some(receiver);
                    receiver_retained = true;
                    break "empty";
                }
                Err(TryRecvError::Disconnected) => {
                    self.path_index.status =
                        PathIndexStatus::Error("background path indexer disconnected".to_string());
                    changed = true;
                    break "disconnected";
                }
            }
        };
        let duration_ms = started_at.elapsed().as_millis();
        if batch_count > 0
            || terminal_state != "empty"
            || duration_ms >= LOG_PATH_INDEX_POLL_THRESHOLD_MS
        {
            debug_log::log(&format!(
                "component=path_index_poll batches={batch_count} entries={entry_count} changed={changed} terminal={terminal_state} receiver_retained={receiver_retained} snapshot_entries={} visible_rows={} query_len={} status={} duration_ms={duration_ms}",
                self.path_index.snapshot.entries.len(),
                self.tree.visible_rows.len(),
                self.path_filter.query.len(),
                describe_path_index_status(&self.path_index.status),
            ));
        }
        Ok(changed)
    }

    pub fn open_content_search(&mut self) -> bool {
        let changed = !self.content_search.active;
        self.content_search.active = true;
        let started_index = self.start_path_index_if_needed();
        changed || started_index
    }

    pub fn close_content_search(&mut self) -> bool {
        if !self.content_search.active {
            return false;
        }
        self.content_search.active = false;
        true
    }

    pub fn append_content_search_char(&mut self, ch: char) -> bool {
        let mut next_query = self.content_search.query.clone();
        next_query.push(ch);
        self.set_content_search_query(&next_query)
    }

    pub fn backspace_content_search(&mut self) -> bool {
        if self.content_search.query.is_empty() {
            return false;
        }
        let mut next_query = self.content_search.query.clone();
        next_query.pop();
        self.set_content_search_query(&next_query)
    }

    pub fn set_content_search_query(&mut self, query: &str) -> bool {
        if self.content_search.query == query {
            return false;
        }

        self.content_search.query = query.to_string();
        self.content_search.generation.0 = self.content_search.generation.0.saturating_add(1);
        self.content_search.status = ContentSearchStatus::Idle;
        self.content_search.status_message = None;
        self.content_search.payload = SearchPayload {
            query: query.to_string(),
            hits: Vec::new(),
        };
        self.content_search.selected_hit_index = None;
        self.content_search.runtime.worker = None;
        true
    }

    pub fn apply_content_search_response(&mut self, response: SearchResponse) -> bool {
        if response.generation != self.content_search.generation {
            return false;
        }

        let next_selected_hit_index = clamp_content_search_selection(
            self.content_search.selected_hit_index,
            response.payload.hits.len(),
        );
        let next_status_message = Some(content_search_status_message(response.payload.hits.len()));
        let changed = self.content_search.status != ContentSearchStatus::Ready
            || self.content_search.payload != response.payload
            || self.content_search.selected_hit_index != next_selected_hit_index
            || self.content_search.status_message != next_status_message;

        self.content_search.status = ContentSearchStatus::Ready;
        self.content_search.status_message = next_status_message;
        self.content_search.payload = response.payload;
        self.content_search.selected_hit_index = next_selected_hit_index;
        changed
    }

    pub fn select_content_search_hit(&mut self, index: usize) -> bool {
        if self.content_search.payload.hits.get(index).is_none() {
            return false;
        }
        if self.content_search.selected_hit_index == Some(index) {
            return false;
        }
        self.content_search.selected_hit_index = Some(index);
        true
    }

    pub fn move_content_search_selection(&mut self, delta: isize) -> bool {
        let hit_count = self.content_search.payload.hits.len();
        if hit_count == 0 {
            return false;
        }

        let current = self.content_search.selected_hit_index.unwrap_or(0);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.saturating_abs() as usize)
        } else {
            current.saturating_add(delta as usize).min(hit_count - 1)
        };
        if next == current {
            return false;
        }

        self.content_search.selected_hit_index = Some(next);
        true
    }

    pub fn selected_content_search_hit(&self) -> Option<&crate::preview::model::SearchHit> {
        let index = self.content_search.selected_hit_index?;
        self.content_search.payload.hits.get(index)
    }

    fn reveal_rel_path_in_tree(&mut self, rel_path: &std::path::Path) -> bool {
        if !self.tree.path_to_id.contains_key(rel_path) {
            indexer::merge_snapshot_into_tree(&mut self.tree, &self.path_index.snapshot);
        }

        let mut changed = false;
        let mut ancestors = Vec::new();
        let mut current = rel_path.parent();
        while let Some(path) = current {
            if path.as_os_str().is_empty() {
                break;
            }
            ancestors.push(path.to_path_buf());
            current = path.parent();
        }
        ancestors.reverse();
        for ancestor in ancestors {
            changed |= self.tree.expand_rel_path(&ancestor);
        }
        changed |= self.tree.select_rel_path(rel_path);
        changed
    }

    pub fn refresh_git_state<B: GitBackend>(&mut self, backend: &B) -> Result<bool> {
        let previous_repo = self.git.repo.clone();
        let previous_status_map = self.git.status_map.clone();
        let previous_error = self.git.last_error.clone();

        match backend.discover_repo(&self.root)? {
            Some(repo) => match backend.status_map(&repo) {
                Ok(status_map) => {
                    self.git.repo = Some(repo);
                    self.git.status_map = status_map;
                    self.git.last_error = None;
                    self.git.initialized = true;
                    self.git.needs_refresh = false;
                }
                Err(err) => {
                    self.git.repo = Some(repo);
                    self.git.status_map.clear();
                    self.git.last_error = Some(err.to_string());
                    self.git.initialized = true;
                    self.git.needs_refresh = false;
                    let changed = self.sync_git_badges()
                        || self.git.repo != previous_repo
                        || self.git.status_map != previous_status_map
                        || self.git.last_error != previous_error;
                    if changed {
                        self.git.generation.0 = self.git.generation.0.saturating_add(1);
                    }
                    return Err(err.into());
                }
            },
            None => {
                self.git.repo = None;
                self.git.status_map.clear();
                self.git.last_error = None;
                self.git.initialized = true;
                self.git.needs_refresh = false;
            }
        }

        let changed = self.sync_git_badges()
            || self.git.repo != previous_repo
            || self.git.status_map != previous_status_map
            || self.git.last_error != previous_error;
        if changed {
            self.git.generation.0 = self.git.generation.0.saturating_add(1);
        }
        Ok(changed)
    }

    pub fn refresh_tree_after_file_op(&mut self, reveal_rel_path: Option<&Path>) -> Result<bool> {
        let visibility = self.tree.visibility_settings();
        let selected_rel_path = reveal_rel_path
            .map(Path::to_path_buf)
            .or_else(|| self.tree.selected_rel_path());
        let pre_filter_selected_rel_path = if self.path_filter.query.is_empty() {
            None
        } else {
            reveal_rel_path.map(Path::to_path_buf).or_else(|| {
                self.tree
                    .pre_filter_selected
                    .and_then(|node_id| self.tree.node(node_id).map(|node| node.rel_path.clone()))
            })
        };
        let pre_filter_scroll_row =
            if self.path_filter.query.is_empty() || reveal_rel_path.is_some() {
                None
            } else {
                self.tree.pre_filter_scroll_row
            };
        let expanded_directories = self.tree.expanded_directory_paths();
        let query = self.path_filter.query.clone();
        let path_filter_active = self.path_filter.active;
        let root = self.root.clone();

        let mut tree = crate::tree::loader::load_root_shallow_with_visibility(&root, visibility)?;
        for rel_path in &expanded_directories {
            let _ = tree.expand_rel_path(rel_path);
        }
        if let Some(rel_path) = pre_filter_selected_rel_path
            .as_deref()
            .or(selected_rel_path.as_deref())
        {
            let _ = tree.select_rel_path(rel_path);
        }
        if let Some(scroll_row) = pre_filter_scroll_row {
            tree.scroll_row = scroll_row.min(tree.base_visible_rows.len().saturating_sub(1));
        } else if reveal_rel_path.is_some() {
            tree.scroll_row = tree
                .selected_row
                .min(tree.base_visible_rows.len().saturating_sub(1));
        }

        self.root = tree.root_abs.clone();
        self.tree = tree;
        self.path_index = PathIndexState::default();
        self.path_filter = PathFilterState::default();

        if !query.is_empty() {
            let _ = self.set_path_filter_query(&query)?;
        }
        self.path_filter.active = path_filter_active;

        if let Some(rel_path) = selected_rel_path.as_deref() {
            let _ = self.reveal_rel_path_in_tree(rel_path);
        }
        let _ = self.reconcile_multi_select_paths();
        Ok(true)
    }

    fn refresh_tree_after_watcher_refresh(&mut self) -> Result<(bool, Option<(PathBuf, PathBuf)>)> {
        let visibility = self.tree.visibility_settings();
        let query_is_active = !self.path_filter.query.is_empty();
        let selected_rel_path = self
            .tree
            .selected_rel_path()
            .and_then(|path| {
                if query_is_active && path.as_path() == Path::new(".") {
                    None
                } else {
                    Some(path)
                }
            })
            .or_else(|| {
                if query_is_active {
                    self.tree.pre_filter_selected.and_then(|node_id| {
                        self.tree.node(node_id).map(|node| node.rel_path.clone())
                    })
                } else {
                    None
                }
            });
        let expanded_directories = self.tree.expanded_directory_paths();
        let query = self.path_filter.query.clone();
        let path_filter_active = self.path_filter.active;
        let previous_scroll_row = if query_is_active {
            self.tree
                .pre_filter_scroll_row
                .unwrap_or(self.tree.scroll_row)
        } else {
            self.tree.scroll_row
        };
        let root = self.root.clone();

        let mut tree = crate::tree::loader::load_root_shallow_with_visibility(&root, visibility)?;
        for rel_path in &expanded_directories {
            let _ = tree.expand_rel_path(rel_path);
        }

        let mut selection_recovery = None;
        if let Some(selected_rel_path) = selected_rel_path.as_deref() {
            if tree.select_rel_path(selected_rel_path) {
                tree.scroll_row =
                    previous_scroll_row.min(tree.base_visible_rows.len().saturating_sub(1));
            } else if let Some(recovered_rel_path) =
                nearest_surviving_selection_path(&tree, selected_rel_path)
            {
                if recovered_rel_path != selected_rel_path {
                    let _ = tree.select_rel_path(&recovered_rel_path);
                    tree.scroll_row = tree
                        .selected_row
                        .min(tree.base_visible_rows.len().saturating_sub(1));
                    selection_recovery =
                        Some((selected_rel_path.to_path_buf(), recovered_rel_path));
                } else {
                    tree.scroll_row =
                        previous_scroll_row.min(tree.base_visible_rows.len().saturating_sub(1));
                }
            } else {
                tree.scroll_row =
                    previous_scroll_row.min(tree.base_visible_rows.len().saturating_sub(1));
            }
        } else {
            tree.scroll_row =
                previous_scroll_row.min(tree.base_visible_rows.len().saturating_sub(1));
        }

        self.root = tree.root_abs.clone();
        self.tree = tree;
        self.path_index.snapshot = indexer::build_snapshot_with_visibility(&root, visibility)?;
        self.path_index.receiver = None;
        self.path_index.status = PathIndexStatus::Ready;

        if !query.is_empty() {
            self.tree.capture_pre_filter_state();
            self.apply_active_path_filter();

            if let Some((missing_rel_path, recovered_rel_path)) = selection_recovery.as_ref() {
                let final_rel_path = if self.tree.select_rel_path(recovered_rel_path) {
                    Some(recovered_rel_path.clone())
                } else {
                    nearest_visible_selection_path(&self.tree, recovered_rel_path).and_then(
                        |visible_rel_path| {
                            self.tree
                                .select_rel_path(&visible_rel_path)
                                .then_some(visible_rel_path)
                        },
                    )
                };

                if let Some(final_rel_path) = final_rel_path {
                    selection_recovery = Some((missing_rel_path.clone(), final_rel_path));
                }
            }
        }
        self.path_filter.active = path_filter_active;
        let _ = self.reconcile_multi_select_paths();

        Ok((true, selection_recovery))
    }

    fn invalidate_preview(&mut self) -> bool {
        let changed = self.preview.source.rel_path.is_some()
            || self.preview.render_cache.is_some()
            || self.preview.cursor_line != 0
            || self.preview.preview_selection_range().is_some()
            || self.preview.payload.image.is_some()
            || self.image_runtime.pending_key.is_some()
            || self.image_runtime.inline_image.is_some()
            || self.preview.payload.mermaid.is_some()
            || self.mermaid_runtime.pending_key.is_some()
            || self.mermaid_runtime.inline_image.is_some();
        self.preview.source.rel_path = None;
        self.preview.render_cache = None;
        self.preview.scroll_row = 0;
        self.preview.cursor_line = 0;
        self.preview.editor_line_hint = None;
        self.preview.clear_preview_selection();
        self.image_runtime.pending_key = None;
        self.image_runtime.inline_image = None;
        self.mermaid_runtime.pending_key = None;
        self.mermaid_runtime.inline_image = None;
        changed
    }

    fn invalidate_content_search_results(&mut self) -> bool {
        let query = self.content_search.query.clone();
        let changed = self.content_search.status != ContentSearchStatus::Idle
            || self.content_search.status_message.is_some()
            || !self.content_search.payload.hits.is_empty()
            || self.content_search.selected_hit_index.is_some()
            || self.content_search.runtime.worker.is_some()
            || !self.content_search.query.trim().is_empty();

        self.content_search.generation.0 = self.content_search.generation.0.saturating_add(1);
        self.content_search.status = ContentSearchStatus::Idle;
        self.content_search.status_message = None;
        self.content_search.payload = SearchPayload {
            query,
            hits: Vec::new(),
        };
        self.content_search.selected_hit_index = None;
        self.content_search.runtime.worker = None;

        changed
    }

    fn selected_path_has_unstaged_diff(&self) -> bool {
        let Some(selected_rel_path) = self.tree.selected_rel_path() else {
            return false;
        };
        let Some(node_id) = self.tree.path_to_id.get(&selected_rel_path).copied() else {
            return false;
        };
        let Some(node) = self.tree.node(node_id) else {
            return false;
        };
        if !matches!(node.kind, NodeKind::File | NodeKind::SymlinkFile) {
            return false;
        }
        let Some(repo) = self.git.repo.as_ref() else {
            return false;
        };

        let backend = LibgitBackend;
        backend
            .diff_for_path(repo, &node.rel_path, DiffMode::Unstaged)
            .map(|diff| !diff.text.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn sync_git_badges(&mut self) -> bool {
        let mut changed = false;
        let node_ids = self
            .tree
            .nodes
            .iter()
            .flatten()
            .map(|node| node.id)
            .collect::<Vec<_>>();

        for node_id in node_ids {
            let next_status = {
                let Some(node) = self.tree.node(node_id) else {
                    continue;
                };
                self.git_status_for_rel_path(&node.rel_path)
            };

            if let Some(node) = self.tree.node_mut(node_id)
                && node.git != next_status
            {
                node.git = next_status;
                changed = true;
            }
        }

        changed
    }

    fn git_status_for_rel_path(&self, rel_path: &std::path::Path) -> GitStatus {
        let mut status = GitStatus::Unmodified;
        for (path, path_status) in &self.git.status_map {
            let matches = if rel_path.as_os_str().is_empty() {
                true
            } else {
                path == rel_path || path.starts_with(rel_path)
            };
            if matches {
                status = combine_git_status(status, git_status_for_path(path_status));
            }
        }
        status
    }

    pub fn ingest_path_index_batch(&mut self, entries: Vec<PathIndexEntry>) -> bool {
        if entries.is_empty() {
            return false;
        }

        let started_at = Instant::now();
        let batch_size = entries.len();
        indexer::merge_entries_into_tree(&mut self.tree, &entries);
        self.path_index.snapshot.entries.extend(entries);
        let indexed_paths = self.path_index.snapshot.entries.len();
        self.path_index.status = PathIndexStatus::Building { indexed_paths };
        if self.path_filter.query.is_empty() {
            self.tree.rebuild_visible_rows();
        } else {
            self.apply_active_path_filter();
        }
        let _ = self.resubmit_content_search_with_current_snapshot();
        let duration_ms = started_at.elapsed().as_millis();
        if duration_ms >= SLOW_PATH_INDEX_BATCH_THRESHOLD_MS {
            debug_log::log(&format!(
                "component=path_index_ingest_slow batch_size={batch_size} snapshot_entries={} base_visible_rows={} visible_rows={} query_len={} duration_ms={duration_ms}",
                self.path_index.snapshot.entries.len(),
                self.tree.base_visible_rows.len(),
                self.tree.visible_rows.len(),
                self.path_filter.query.len(),
            ));
        }
        true
    }

    pub fn complete_path_index(&mut self) {
        self.path_index.receiver = None;
        self.path_index.status = PathIndexStatus::Ready;
        let _ = self.resubmit_content_search_with_current_snapshot();
    }

    pub fn toggle_show_hidden(&mut self) -> Result<bool> {
        let mut visibility = self.tree.visibility_settings();
        visibility.show_hidden = !visibility.show_hidden;
        self.rebuild_for_visibility(visibility)
    }

    pub fn toggle_respect_gitignore(&mut self) -> Result<bool> {
        let mut visibility = self.tree.visibility_settings();
        visibility.respect_gitignore = !visibility.respect_gitignore;
        self.rebuild_for_visibility(visibility)
    }

    fn apply_active_path_filter(&mut self) {
        crate::tree::filter::apply_query(
            &mut self.tree,
            &self.path_index.snapshot,
            &self.path_filter.query,
        );
    }

    fn rebuild_for_visibility(&mut self, visibility: VisibilitySettings) -> Result<bool> {
        let previous_visibility = self.tree.visibility_settings();
        if previous_visibility == visibility {
            return Ok(false);
        }

        let rebuild_started_at = Instant::now();
        let selected_rel_path = self.tree.selected_rel_path();
        let pre_filter_selected_rel_path = if self.path_filter.query.is_empty() {
            None
        } else {
            self.tree
                .pre_filter_selected
                .and_then(|node_id| self.tree.node(node_id).map(|node| node.rel_path.clone()))
        };
        let pre_filter_scroll_row = if self.path_filter.query.is_empty() {
            None
        } else {
            self.tree.pre_filter_scroll_row
        };
        let expanded_directories = self.tree.expanded_directory_paths();
        let query = self.path_filter.query.clone();
        let path_filter_active = self.path_filter.active;
        let root = self.root.clone();
        let previous_snapshot_entries = self.path_index.snapshot.entries.len();
        let previous_visible_rows = self.tree.visible_rows.len();
        let previous_receiver_active = self.path_index.receiver.is_some();
        debug_log::log(&format!(
            "component=visibility_rebuild phase=start show_hidden_before={} show_hidden_after={} respect_gitignore_before={} respect_gitignore_after={} query_active={} expanded_dirs={} previous_snapshot_entries={previous_snapshot_entries} previous_visible_rows={previous_visible_rows} previous_receiver_active={previous_receiver_active} selected={}",
            previous_visibility.show_hidden,
            visibility.show_hidden,
            previous_visibility.respect_gitignore,
            visibility.respect_gitignore,
            !query.is_empty(),
            expanded_directories.len(),
            selected_rel_path
                .as_deref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| ".".to_string())
        ));

        let mut tree = crate::tree::loader::load_root_shallow_with_visibility(&root, visibility)?;
        for rel_path in &expanded_directories {
            let _ = tree.expand_rel_path(rel_path);
        }
        if let Some(rel_path) = pre_filter_selected_rel_path
            .as_deref()
            .or(selected_rel_path.as_deref())
        {
            let _ = tree.select_rel_path(rel_path);
        }
        if let Some(scroll_row) = pre_filter_scroll_row {
            tree.scroll_row = scroll_row.min(tree.base_visible_rows.len().saturating_sub(1));
        }

        self.root = tree.root_abs.clone();
        self.tree = tree;
        self.path_filter = PathFilterState::default();
        self.path_index = PathIndexState::default();
        self.preview.source.rel_path = None;
        self.preview.render_cache = None;
        let _ = self.sync_git_badges();

        if !query.is_empty() {
            let _ = self.set_path_filter_query(&query)?;
            if let Some(rel_path) = selected_rel_path.as_deref() {
                let _ = self.tree.select_rel_path(rel_path);
            }
        }
        self.path_filter.active = path_filter_active;
        let _ = self.reconcile_multi_select_paths();
        debug_log::log(&format!(
            "component=visibility_rebuild phase=done show_hidden={} respect_gitignore={} query_active={} expanded_dirs={} base_rows={} receiver_active={} snapshot_entries={} duration_ms={}",
            visibility.show_hidden,
            visibility.respect_gitignore,
            !query.is_empty(),
            expanded_directories.len(),
            self.tree.base_visible_rows.len(),
            self.path_index.receiver.is_some(),
            self.path_index.snapshot.entries.len(),
            rebuild_started_at.elapsed().as_millis()
        ));

        Ok(true)
    }

    fn start_path_index_if_needed(&mut self) -> bool {
        if self.path_index.receiver.is_some()
            || matches!(self.path_index.status, PathIndexStatus::Ready)
        {
            return false;
        }

        self.path_index =
            PathIndexState::building(self.root.clone(), self.tree.visibility_settings());
        true
    }

    fn resubmit_content_search_with_current_snapshot(&mut self) -> bool {
        if !self.content_search.active {
            return false;
        }
        let query = self.content_search.query.trim();
        if query.is_empty() {
            return false;
        }
        if self.path_index.snapshot.entries.is_empty() && self.path_index.receiver.is_some() {
            let changed = self.content_search.status != ContentSearchStatus::Searching
                || self.content_search.status_message.as_deref() != Some("searching repository");
            self.content_search.status = ContentSearchStatus::Searching;
            self.content_search.status_message = Some("searching repository".to_string());
            return changed;
        }
        if self.content_search.runtime.worker.is_none() {
            self.content_search.runtime.worker = Some(start_background_content_search());
        }
        let Some(worker) = self.content_search.runtime.worker.as_ref() else {
            self.content_search.status = ContentSearchStatus::Error;
            self.content_search.status_message =
                Some("content search worker unavailable".to_string());
            return true;
        };

        let submitted = worker.submit(ContentSearchRequest {
            generation: self.content_search.generation,
            root_abs: self.root.clone(),
            snapshot: self.path_index.snapshot.clone(),
            query: self.content_search.query.clone(),
            max_results: DEFAULT_CONTENT_SEARCH_MAX_RESULTS,
        });
        if !submitted {
            self.content_search.status = ContentSearchStatus::Error;
            self.content_search.status_message =
                Some("content search worker unavailable".to_string());
            self.content_search.runtime.worker = None;
            return true;
        }

        self.content_search.status = ContentSearchStatus::Searching;
        self.content_search.status_message = Some("searching repository".to_string());
        true
    }

    fn refresh_preview(&mut self, config: &crate::config::PreviewConfig) -> bool {
        if self.mode == ContextMode::SearchResults {
            return self.refresh_search_results_preview();
        }

        let Some(selected_rel_path) = self.tree.selected_rel_path() else {
            return false;
        };
        if self.preview.source.rel_path.as_ref() == Some(&selected_rel_path)
            && self.preview.source.context_mode == self.mode
        {
            return false;
        }

        let Some(node_id) = self.tree.path_to_id.get(&selected_rel_path).copied() else {
            return false;
        };
        let Some(node) = self.tree.node(node_id) else {
            return false;
        };
        let next_scroll_row = self.preview.selected_line_start.take().unwrap_or(0);
        self.preview.selected_line_end = None;
        self.preview.cursor_line = next_scroll_row;

        self.preview.generation.0 = self.preview.generation.0.saturating_add(1);
        self.preview.source.rel_path = Some(selected_rel_path);
        self.preview.source.context_mode = self.mode;
        let (payload, editor_line_hint) = match self.mode {
            ContextMode::Preview => (
                crate::preview::loader::load_preview(&self.root, node, config),
                Some(next_scroll_row.saturating_add(1).max(1)),
            ),
            ContextMode::Diff => self.load_diff_preview(node),
            ContextMode::Blame => (
                self.load_context_placeholder(
                    "Blame",
                    "Git blame is not implemented in this slice.",
                ),
                None,
            ),
            ContextMode::Info => (
                self.load_context_placeholder("Info", "Git info mode is not implemented yet."),
                None,
            ),
            ContextMode::SearchResults => (
                self.load_context_placeholder(
                    "Search Results",
                    "Search results mode is not implemented yet.",
                ),
                None,
            ),
        };
        self.preview.payload = payload;
        self.preview.render_cache = None;
        self.preview.scroll_row = next_scroll_row;
        self.preview.editor_line_hint = editor_line_hint;
        self.image_runtime.pending_key = None;
        self.image_runtime.inline_image = None;
        self.mermaid_runtime.pending_key = None;
        self.mermaid_runtime.inline_image = None;
        let _ = self.queue_image_render(config);
        let _ = self.queue_mermaid_render(config);
        true
    }

    fn queue_image_render(&mut self, config: &crate::config::PreviewConfig) -> bool {
        let Some(image) = self.preview.payload.image.as_ref() else {
            self.image_runtime.pending_key = None;
            self.image_runtime.inline_image = None;
            return false;
        };
        let Some(selected_rel_path) = self.preview.source.rel_path.as_ref() else {
            self.image_runtime.pending_key = None;
            self.image_runtime.inline_image = None;
            return false;
        };
        let abs_path = self.root.join(selected_rel_path);
        let Ok(metadata) = fs::metadata(&abs_path) else {
            if let Some(image) = self.preview.payload.image.as_mut() {
                let changed = image.display != crate::preview::model::ImageDisplay::Summary
                    || image.status != "Image preview unavailable; showing metadata summary";
                image.display = crate::preview::model::ImageDisplay::Summary;
                image.status = "Image preview unavailable; showing metadata summary".to_string();
                image.body_lines.clear();
                self.image_runtime.pending_key = None;
                self.image_runtime.inline_image = None;
                return changed;
            }
            return false;
        };
        let request = build_image_render_request(
            self.preview.generation,
            &abs_path,
            image.format_label.clone(),
            &metadata,
            config,
            crate::preview::mermaid::inline_images_supported(),
        );
        if self.image_runtime.pending_key.as_ref() == Some(&request.key) {
            return false;
        }

        let worker = self
            .image_runtime
            .worker
            .get_or_insert_with(start_background_image_render);
        if !worker.submit(request.clone()) {
            if let Some(image) = self.preview.payload.image.as_mut() {
                let changed = image.display != crate::preview::model::ImageDisplay::Summary
                    || image.status != "Image preview worker unavailable; showing metadata summary";
                image.display = crate::preview::model::ImageDisplay::Summary;
                image.status =
                    "Image preview worker unavailable; showing metadata summary".to_string();
                image.body_lines.clear();
                self.image_runtime.pending_key = None;
                self.image_runtime.inline_image = None;
                return changed;
            }
            return false;
        }

        self.image_runtime.pending_key = Some(request.key);
        false
    }

    fn refresh_search_results_preview(&mut self) -> bool {
        let payload = self.load_search_results_preview();
        let next_scroll_row = self
            .content_search
            .selected_hit_index
            .unwrap_or_default()
            .saturating_sub(1);
        let changed = self.preview.source.context_mode != ContextMode::SearchResults
            || self.preview.source.rel_path.is_some()
            || self.preview.payload != payload
            || self.preview.scroll_row != next_scroll_row
            || self.preview.preview_selection_range().is_some();
        if !changed {
            return false;
        }

        self.preview.generation.0 = self.preview.generation.0.saturating_add(1);
        self.preview.source.rel_path = None;
        self.preview.source.context_mode = ContextMode::SearchResults;
        self.preview.payload = payload;
        self.preview.render_cache = None;
        self.preview.scroll_row = next_scroll_row;
        self.preview.cursor_line = next_scroll_row;
        self.preview.editor_line_hint = self
            .content_search
            .selected_hit_index
            .and_then(|index| self.content_search.payload.hits.get(index))
            .map(|hit| hit.line.max(1));
        self.preview.clear_preview_selection();
        true
    }

    fn queue_mermaid_render(&mut self, config: &crate::config::PreviewConfig) -> bool {
        let Some(mermaid) = self.preview.payload.mermaid.as_ref() else {
            self.mermaid_runtime.pending_key = None;
            self.mermaid_runtime.inline_image = None;
            return false;
        };

        let discovery = discover_renderers(config);
        if discovery.rich_command.is_none() && discovery.ascii_helper_command.is_none() {
            if let Some(mermaid) = self.preview.payload.mermaid.as_mut() {
                let changed = mermaid.display != crate::preview::model::MermaidDisplay::RawSource
                    || mermaid.status != "Mermaid renderer unavailable; showing raw source";
                mermaid.display = crate::preview::model::MermaidDisplay::RawSource;
                mermaid.status = "Mermaid renderer unavailable; showing raw source".to_string();
                self.mermaid_runtime.pending_key = None;
                self.mermaid_runtime.inline_image = None;
                return changed;
            }
            return false;
        }

        let request = build_render_request(
            self.preview.generation,
            mermaid,
            config,
            discovery,
            crate::preview::mermaid::inline_images_supported(),
        );
        if self.mermaid_runtime.pending_key.as_ref() == Some(&request.key) {
            return false;
        }

        let worker = self
            .mermaid_runtime
            .worker
            .get_or_insert_with(start_background_mermaid_render);
        if !worker.submit(request.clone()) {
            if let Some(mermaid) = self.preview.payload.mermaid.as_mut() {
                let changed = mermaid.display != crate::preview::model::MermaidDisplay::RawSource
                    || mermaid.status != "Mermaid renderer worker unavailable; showing raw source";
                mermaid.display = crate::preview::model::MermaidDisplay::RawSource;
                mermaid.status =
                    "Mermaid renderer worker unavailable; showing raw source".to_string();
                self.mermaid_runtime.pending_key = None;
                self.mermaid_runtime.inline_image = None;
                return changed;
            }
            return false;
        }

        self.mermaid_runtime.pending_key = Some(request.key);
        false
    }

    fn poll_mermaid_render(&mut self) -> bool {
        let (latest_response, disconnected) = {
            let Some(worker) = self.mermaid_runtime.worker.as_ref() else {
                return false;
            };
            let mut latest_response = None;
            let mut disconnected = false;
            loop {
                match worker.try_recv() {
                    Ok(response) => latest_response = Some(response),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            (latest_response, disconnected)
        };

        let mut changed = false;
        if let Some(response) = latest_response {
            changed |= self.apply_mermaid_render_response(response);
        }
        if disconnected {
            self.mermaid_runtime.worker = None;
            self.mermaid_runtime.pending_key = None;
            if let Some(mermaid) = self.preview.payload.mermaid.as_mut() {
                let had_error =
                    mermaid.status == "Mermaid renderer worker disconnected; showing raw source";
                mermaid.display = crate::preview::model::MermaidDisplay::RawSource;
                mermaid.status =
                    "Mermaid renderer worker disconnected; showing raw source".to_string();
                self.mermaid_runtime.inline_image = None;
                changed |= !had_error;
            }
        }

        changed
    }

    fn apply_mermaid_render_response(&mut self, response: MermaidRenderResponse) -> bool {
        if self.mermaid_runtime.pending_key.as_ref() != Some(&response.key) {
            return false;
        }
        let Some(mermaid) = self.preview.payload.mermaid.as_mut() else {
            return false;
        };

        self.mermaid_runtime.pending_key = None;
        mermaid.status = response.status;

        match response.outcome {
            MermaidRenderOutcome::Image(image) => {
                mermaid.display = crate::preview::model::MermaidDisplay::Image;
                mermaid.body_lines.clear();
                self.mermaid_runtime.inline_image = Some(image);
            }
            MermaidRenderOutcome::Ascii(lines) => {
                mermaid.display = crate::preview::model::MermaidDisplay::Ascii;
                mermaid.body_lines = lines;
                self.mermaid_runtime.inline_image = None;
            }
            MermaidRenderOutcome::RawSource => {
                mermaid.display = crate::preview::model::MermaidDisplay::RawSource;
                if mermaid.body_lines.is_empty() {
                    mermaid.body_lines = mermaid
                        .source
                        .raw_source
                        .lines()
                        .map(ToOwned::to_owned)
                        .collect();
                }
                self.mermaid_runtime.inline_image = None;
            }
        }

        self.preview.generation.0 = self.preview.generation.0.saturating_add(1);
        self.preview.render_cache = None;
        true
    }

    fn poll_image_render(&mut self) -> bool {
        let (latest_response, disconnected) = {
            let Some(worker) = self.image_runtime.worker.as_ref() else {
                return false;
            };
            let mut latest_response = None;
            let mut disconnected = false;
            loop {
                match worker.try_recv() {
                    Ok(response) => latest_response = Some(response),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            (latest_response, disconnected)
        };

        let mut changed = false;
        if let Some(response) = latest_response {
            changed |= self.apply_image_render_response(response);
        }
        if disconnected {
            self.image_runtime.worker = None;
            self.image_runtime.pending_key = None;
            if let Some(image) = self.preview.payload.image.as_mut() {
                let had_error =
                    image.status == "Image preview worker disconnected; showing metadata summary";
                image.display = crate::preview::model::ImageDisplay::Summary;
                image.status =
                    "Image preview worker disconnected; showing metadata summary".to_string();
                image.body_lines.clear();
                self.image_runtime.inline_image = None;
                changed |= !had_error;
            }
        }

        changed
    }

    fn apply_image_render_response(&mut self, response: ImageRenderResponse) -> bool {
        if self.image_runtime.pending_key.as_ref() != Some(&response.key) {
            return false;
        }
        let Some(image) = self.preview.payload.image.as_mut() else {
            return false;
        };

        self.image_runtime.pending_key = None;
        image.status = response.status;
        image.dimensions = response.dimensions;
        sync_header_metadata_dimensions(&mut self.preview.payload.header, response.dimensions);

        match response.outcome {
            ImageRenderOutcome::Inline(inline) => {
                image.display = crate::preview::model::ImageDisplay::Inline;
                image.body_lines.clear();
                self.image_runtime.inline_image = Some(inline);
            }
            ImageRenderOutcome::Summary => {
                image.display = crate::preview::model::ImageDisplay::Summary;
                image.body_lines.clear();
                self.image_runtime.inline_image = None;
            }
        }

        self.preview.generation.0 = self.preview.generation.0.saturating_add(1);
        self.preview.render_cache = None;
        true
    }

    fn load_diff_preview(&self, node: &Node) -> (PreviewPayload, Option<usize>) {
        if !matches!(node.kind, NodeKind::File | NodeKind::SymlinkFile) {
            return (
                self.load_context_placeholder("Diff", "Git diff unavailable: select a file."),
                None,
            );
        }

        let Some(repo) = self.git.repo.as_ref() else {
            return (
                self.load_context_placeholder(
                    "Diff",
                    "Git diff unavailable: not inside a git repository.",
                ),
                None,
            );
        };

        let backend = LibgitBackend;
        match backend.diff_for_path(repo, &node.rel_path, DiffMode::Unstaged) {
            Ok(diff) if !diff.text.trim().is_empty() => (
                PreviewPayload {
                    title: format!("Diff {}", node.rel_path.display()),
                    header: PreviewHeader::default(),
                    lines: diff.text.lines().map(str::to_string).collect(),
                    markdown: None,
                    image: None,
                    mermaid: None,
                },
                diff.first_changed_line,
            ),
            Ok(_) => (
                self.load_context_placeholder(
                    "Diff",
                    "Git diff unavailable: no unstaged diff for selected path.",
                ),
                None,
            ),
            Err(err) => (
                self.load_context_placeholder("Diff", &format!("Git diff unavailable: {err}")),
                None,
            ),
        }
    }

    fn load_context_placeholder(&self, title: &str, message: &str) -> PreviewPayload {
        PreviewPayload {
            title: title.to_string(),
            header: PreviewHeader::default(),
            lines: vec![message.to_string()],
            markdown: None,
            image: None,
            mermaid: None,
        }
    }

    fn load_search_results_preview(&self) -> PreviewPayload {
        let mut lines = Vec::new();
        if !self.content_search.query.is_empty() {
            lines.push(format!("Query: {}", self.content_search.query));
        }
        if let Some(status_message) = self.content_search.status_message.as_deref() {
            lines.push(format!("Status: {status_message}"));
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }

        if self.content_search.payload.hits.is_empty() {
            let message = if self.content_search.query.is_empty() {
                "Enter a query and press Enter.".to_string()
            } else if matches!(self.content_search.status, ContentSearchStatus::Searching) {
                "Searching repository...".to_string()
            } else {
                format!("No results for {}", self.content_search.query)
            };
            lines.push(message);
        } else {
            for (index, hit) in self.content_search.payload.hits.iter().enumerate() {
                let marker = if self.content_search.selected_hit_index == Some(index) {
                    ">"
                } else {
                    " "
                };
                lines.push(format!(
                    "{marker} {}:{}  {}",
                    hit.path, hit.line, hit.excerpt
                ));
            }
        }

        PreviewPayload {
            title: "Search Results".to_string(),
            header: PreviewHeader::default(),
            lines,
            markdown: None,
            image: None,
            mermaid: None,
        }
    }

    pub fn active_inline_preview_image(&self) -> Option<(&'static str, &[u8])> {
        if let Some(image) = self.image_runtime.inline_image.as_ref() {
            return Some(("grove-image.png", image.png_bytes.as_slice()));
        }
        self.mermaid_runtime
            .inline_image
            .as_ref()
            .map(|image| ("grove-mermaid.png", image.png_bytes.as_slice()))
    }
}

fn sync_header_metadata_dimensions(header: &mut PreviewHeader, dimensions: Option<(u32, u32)>) {
    let existing_index = header
        .metadata
        .iter()
        .position(|item| item.label == "Dimensions");
    match (existing_index, dimensions) {
        (Some(index), Some((width, height))) => {
            header.metadata[index].value = format!("{width} x {height}");
        }
        (None, Some((width, height))) => {
            header
                .metadata
                .push(crate::preview::model::PreviewMetadataItem {
                    label: "Dimensions".to_string(),
                    value: format!("{width} x {height}"),
                })
        }
        (Some(index), None) => {
            header.metadata.remove(index);
        }
        (None, None) => {}
    }
}

#[derive(Debug, Clone, Default)]
pub struct BridgeState {
    pub connected: bool,
    pub instance_id: Option<String>,
    pub ai_target_session_id: Option<String>,
    pub editor_target_session_id: Option<String>,
    pub session_summaries: Vec<SessionSummary>,
    pub resolved_targets: Option<TargetResolution>,
}

#[derive(Debug, Default)]
pub struct MermaidPreviewRuntime {
    pub worker: Option<MermaidRenderWorker>,
    pub pending_key: Option<MermaidRenderKey>,
    pub inline_image: Option<MermaidInlineImage>,
}

#[derive(Debug, Default)]
pub struct ImagePreviewRuntime {
    pub worker: Option<ImageRenderWorker>,
    pub pending_key: Option<ImageRenderKey>,
    pub inline_image: Option<ImageInlineImage>,
}

#[derive(Debug, Clone, Default)]
pub struct PreviewState {
    pub generation: PreviewGeneration,
    pub source: PreviewSource,
    pub payload: PreviewPayload,
    pub render_cache: Option<PreviewRenderCache>,
    pub scroll_row: usize,
    pub cursor_line: usize,
    pub editor_line_hint: Option<usize>,
    pub selected_line_start: Option<usize>,
    pub selected_line_end: Option<usize>,
}

impl PreviewState {
    pub fn preview_selection_range(&self) -> Option<(usize, usize)> {
        match (self.selected_line_start, self.selected_line_end) {
            (Some(start), Some(end)) => Some((start.min(end), start.max(end))),
            _ => None,
        }
    }

    pub fn clear_preview_selection(&mut self) -> bool {
        let changed = self.selected_line_start.is_some() || self.selected_line_end.is_some();
        self.selected_line_start = None;
        self.selected_line_end = None;
        changed
    }

    pub fn clamp_preview_selection(&mut self, line_count: usize) -> bool {
        let max_line = line_count.saturating_sub(1);
        let mut changed = false;

        let next_cursor_line = self.cursor_line.min(max_line);
        if next_cursor_line != self.cursor_line {
            self.cursor_line = next_cursor_line;
            changed = true;
        }

        match (self.selected_line_start, self.selected_line_end) {
            (Some(start), Some(end)) => {
                let start = start.min(max_line);
                let end = end.min(max_line);
                if self.selected_line_start != Some(start) {
                    self.selected_line_start = Some(start);
                    changed = true;
                }
                if self.selected_line_end != Some(end) {
                    self.selected_line_end = Some(end);
                    changed = true;
                }
            }
            (None, None) => {}
            _ => {
                changed |= self.clear_preview_selection();
            }
        }

        changed
    }

    pub fn set_cursor_line(&mut self, line: usize, line_count: usize) -> bool {
        let max_line = line_count.saturating_sub(1);
        let next_line = line.min(max_line);
        let changed = self.cursor_line != next_line;
        self.cursor_line = next_line;
        self.clear_preview_selection() || changed
    }

    pub fn extend_selection_by(&mut self, delta: isize, line_count: usize) -> bool {
        let max_line = line_count.saturating_sub(1);
        let next_line = if delta.is_negative() {
            self.cursor_line
                .saturating_sub(delta.saturating_abs() as usize)
        } else {
            self.cursor_line
                .saturating_add(delta as usize)
                .min(max_line)
        };
        if next_line == self.cursor_line {
            return false;
        }
        let changed = self.selected_line_start.is_none()
            || self.selected_line_end.is_none()
            || self.selected_line_end != Some(next_line);
        self.selected_line_start.get_or_insert(self.cursor_line);
        self.selected_line_end = Some(next_line);
        self.cursor_line = next_line;
        changed
    }

    fn refresh_render_cache(&mut self, preview_width: u16) -> bool {
        let presentation = self.presentation();
        crate::preview::render::refresh_cache(
            &mut self.render_cache,
            self.generation,
            &self.payload,
            presentation,
            preview_width,
        )
    }

    fn presentation(&self) -> PreviewPresentation {
        if self.source.context_mode == ContextMode::Diff {
            return PreviewPresentation::Diff;
        }

        if let Some(image) = self.payload.image.as_ref() {
            return match image.display {
                crate::preview::model::ImageDisplay::Pending => PreviewPresentation::ImagePending,
                crate::preview::model::ImageDisplay::Inline => PreviewPresentation::ImageInline,
                crate::preview::model::ImageDisplay::Summary => PreviewPresentation::ImageSummary,
            };
        }

        match self.payload.mermaid.as_ref().map(|mermaid| mermaid.display) {
            Some(crate::preview::model::MermaidDisplay::Pending) => {
                PreviewPresentation::MermaidPending
            }
            Some(crate::preview::model::MermaidDisplay::Ascii) => PreviewPresentation::MermaidAscii,
            Some(crate::preview::model::MermaidDisplay::Image) => PreviewPresentation::MermaidImage,
            Some(crate::preview::model::MermaidDisplay::RawSource) => {
                PreviewPresentation::MermaidRawSource
            }
            None => PreviewPresentation::Standard,
        }
    }

    fn clamp_scroll(&mut self, preview_width: u16, viewport_height: usize) -> bool {
        let max_scroll = self.max_scroll_row(preview_width, viewport_height);
        let next = self.scroll_row.min(max_scroll);
        if next == self.scroll_row {
            return false;
        }
        self.scroll_row = next;
        true
    }

    fn scroll_by(&mut self, delta: isize, preview_width: u16, viewport_height: usize) -> bool {
        let max_scroll = self.max_scroll_row(preview_width, viewport_height);
        let next = if delta.is_negative() {
            self.scroll_row
                .saturating_sub(delta.saturating_abs() as usize)
        } else {
            self.scroll_row
                .saturating_add(delta as usize)
                .min(max_scroll)
        };

        if next == self.scroll_row {
            return false;
        }
        self.scroll_row = next;
        true
    }

    fn scroll_to_top(&mut self) -> bool {
        if self.scroll_row == 0 {
            return false;
        }
        self.scroll_row = 0;
        true
    }

    fn scroll_to_bottom(&mut self, preview_width: u16, viewport_height: usize) -> bool {
        let next = self.max_scroll_row(preview_width, viewport_height);
        if next == self.scroll_row {
            return false;
        }
        self.scroll_row = next;
        true
    }

    fn max_scroll_row(&self, preview_width: u16, viewport_height: usize) -> usize {
        let _ = preview_width;
        let total_lines = crate::preview::render::line_count_from_cache(self.render_cache.as_ref());
        total_lines.saturating_sub(viewport_height.max(1))
    }
}

#[derive(Debug, Clone, Default)]
pub struct PathFilterState {
    pub query: String,
    pub active: bool,
}

#[derive(Debug)]
pub struct PathIndexState {
    pub status: PathIndexStatus,
    pub snapshot: PathIndexSnapshot,
    pub receiver: Option<Receiver<PathIndexEvent>>,
}

impl Default for PathIndexState {
    fn default() -> Self {
        Self {
            status: PathIndexStatus::Idle,
            snapshot: PathIndexSnapshot::default(),
            receiver: None,
        }
    }
}

impl PathIndexState {
    fn building(root_abs: PathBuf, visibility: VisibilitySettings) -> Self {
        Self {
            status: PathIndexStatus::Building { indexed_paths: 0 },
            snapshot: PathIndexSnapshot::default(),
            receiver: Some(indexer::start_background_index_with_visibility(
                root_abs, visibility,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PathIndexStatus {
    #[default]
    Idle,
    Building {
        indexed_paths: usize,
    },
    Ready,
    Error(String),
}

fn describe_path_index_status(status: &PathIndexStatus) -> String {
    match status {
        PathIndexStatus::Idle => "idle".to_string(),
        PathIndexStatus::Building { indexed_paths } => {
            format!("building(indexed_paths={indexed_paths})")
        }
        PathIndexStatus::Ready => "ready".to_string(),
        PathIndexStatus::Error(message) => format!("error({message})"),
    }
}

fn load_directory_picker_entries(
    current_dir: &Path,
    entry_mode: DirectoryPickerEntryMode,
    visibility: VisibilitySettings,
) -> std::io::Result<Vec<DirectoryPickerEntry>> {
    let current_dir = fs::canonicalize(current_dir)?;
    let mut entries = Vec::new();

    entries.push(DirectoryPickerEntry {
        path: current_dir.clone(),
        label: ".".to_string(),
        is_parent: false,
    });

    if let Some(parent) = current_dir.parent() {
        entries.push(DirectoryPickerEntry {
            path: normalize_root_label_path(parent),
            label: "..".to_string(),
            is_parent: true,
        });
    }

    let child_paths = if visibility.respect_gitignore {
        filtered_directory_picker_paths(&current_dir, entry_mode, visibility)?
    } else {
        raw_directory_picker_paths(&current_dir, entry_mode, visibility)?
    };

    entries.extend(child_paths.into_iter().filter_map(|path| {
        let label = path.file_name()?.to_string_lossy().to_string();
        Some(DirectoryPickerEntry {
            path,
            label,
            is_parent: false,
        })
    }));

    Ok(entries)
}

fn filtered_directory_picker_paths(
    current_dir: &Path,
    entry_mode: DirectoryPickerEntryMode,
    visibility: VisibilitySettings,
) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let walker = crate::tree::walk::build_directory_walker(current_dir, current_dir, visibility);
    for result in walker.build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path == current_dir || path.parent() != Some(current_dir) {
            continue;
        }
        if directory_picker_entry_matches(path, entry_mode)? {
            paths.push(path.to_path_buf());
        }
    }
    Ok(paths)
}

fn raw_directory_picker_paths(
    current_dir: &Path,
    entry_mode: DirectoryPickerEntryMode,
    visibility: VisibilitySettings,
) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(current_dir)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            visibility.show_hidden
                || path
                    .file_name()
                    .is_none_or(|file_name| !file_name.to_string_lossy().starts_with('.'))
        })
        .filter_map(
            |path| match directory_picker_entry_matches(&path, entry_mode) {
                Ok(true) => Some(Ok(path)),
                Ok(false) => None,
                Err(err) => Some(Err(err)),
            },
        )
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    Ok(paths)
}

fn directory_picker_entry_matches(
    path: &Path,
    entry_mode: DirectoryPickerEntryMode,
) -> std::io::Result<bool> {
    let file_type = fs::symlink_metadata(path)?.file_type();
    let is_dir = if file_type.is_symlink() {
        fs::metadata(path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false)
    } else {
        file_type.is_dir()
    };
    let is_file = if file_type.is_symlink() {
        fs::metadata(path)
            .map(|metadata| metadata.is_file())
            .unwrap_or(true)
    } else {
        file_type.is_file()
    };

    Ok(match entry_mode {
        DirectoryPickerEntryMode::DirectoriesOnly => is_dir,
        DirectoryPickerEntryMode::FilesOnly => is_file,
        DirectoryPickerEntryMode::FilesAndDirectories => is_dir || is_file,
    })
}

fn root_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn is_root_rel_path(rel_path: &Path) -> bool {
    rel_path.as_os_str().is_empty() || rel_path == Path::new(".")
}

fn rel_path_exists_under_root(root: &Path, rel_path: &Path) -> bool {
    match fs::symlink_metadata(root.join(rel_path)) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn push_unique_root(roots: &mut Vec<PathBuf>, path: &Path) {
    if roots.iter().all(|existing| existing != path) {
        roots.push(path.to_path_buf());
    }
}

fn normalize_root_label_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let Some(parent) = path.parent() else {
        return path.to_path_buf();
    };
    let normalized_parent = normalize_root_label_path(parent);
    match path.file_name() {
        Some(file_name) => normalized_parent.join(file_name),
        None => normalized_parent,
    }
}

fn nearest_surviving_root_path(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if current.exists() {
            return normalize_root_label_path(current);
        }
        let Some(parent) = current.parent() else {
            return path.to_path_buf();
        };
        current = parent;
    }
}

fn nearest_surviving_selection_path(tree: &TreeState, missing_rel_path: &Path) -> Option<PathBuf> {
    if tree.path_to_id.contains_key(missing_rel_path) {
        return Some(missing_rel_path.to_path_buf());
    }

    let mut branch = missing_rel_path.to_path_buf();
    loop {
        let Some(parent_rel) = branch.parent() else {
            return tree
                .path_to_id
                .contains_key(Path::new("."))
                .then(|| PathBuf::from("."));
        };
        let parent_rel = normalize_rel_path(parent_rel);
        let Some(parent_id) = tree.path_to_id.get(&parent_rel).copied() else {
            branch = parent_rel;
            continue;
        };

        if let Some(recovered_rel_path) = nearest_surviving_child(tree, parent_id, &branch) {
            return Some(recovered_rel_path);
        }
        return Some(parent_rel);
    }
}

fn nearest_visible_selection_path(tree: &TreeState, missing_rel_path: &Path) -> Option<PathBuf> {
    if let Some(node_id) = tree.path_to_id.get(missing_rel_path).copied()
        && is_node_visible(tree, node_id)
    {
        return Some(missing_rel_path.to_path_buf());
    }

    let mut branch = missing_rel_path.to_path_buf();
    loop {
        let Some(parent_rel) = branch.parent() else {
            return if is_node_visible(tree, tree.root_id) {
                Some(PathBuf::from("."))
            } else {
                None
            };
        };
        let parent_rel = normalize_rel_path(parent_rel);
        let Some(parent_id) = tree.path_to_id.get(&parent_rel).copied() else {
            branch = parent_rel;
            continue;
        };

        if is_node_visible(tree, parent_id)
            && let Some(recovered_rel_path) = nearest_visible_child(tree, parent_id, &branch)
        {
            return Some(recovered_rel_path);
        }
        branch = parent_rel;
    }
}

fn nearest_surviving_child(
    tree: &TreeState,
    parent_id: crate::tree::model::NodeId,
    missing_rel_path: &Path,
) -> Option<PathBuf> {
    let parent = tree.node(parent_id)?;
    let target_name = missing_rel_path
        .file_name()?
        .to_string_lossy()
        .to_lowercase();
    let children = parent
        .children
        .iter()
        .filter_map(|child_id| tree.node(*child_id))
        .collect::<Vec<_>>();

    if children.is_empty() {
        return None;
    }

    let insertion_index = children.partition_point(|child| child.name.to_lowercase() < target_name);
    if let Some(child) = children.get(insertion_index) {
        return Some(child.rel_path.clone());
    }
    if insertion_index > 0 {
        return children
            .get(insertion_index - 1)
            .map(|child| child.rel_path.clone());
    }
    None
}

fn nearest_visible_child(
    tree: &TreeState,
    parent_id: crate::tree::model::NodeId,
    missing_rel_path: &Path,
) -> Option<PathBuf> {
    let parent = tree.node(parent_id)?;
    let target_name = missing_rel_path
        .file_name()?
        .to_string_lossy()
        .to_lowercase();
    let children = parent
        .children
        .iter()
        .filter_map(|child_id| {
            let child = tree.node(*child_id)?;
            if is_node_visible(tree, child.id) {
                Some(child)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if children.is_empty() {
        return None;
    }

    let insertion_index = children.partition_point(|child| child.name.to_lowercase() < target_name);
    if let Some(child) = children.get(insertion_index) {
        return Some(child.rel_path.clone());
    }
    if insertion_index > 0 {
        return children
            .get(insertion_index - 1)
            .map(|child| child.rel_path.clone());
    }
    None
}

fn is_node_visible(tree: &TreeState, node_id: crate::tree::model::NodeId) -> bool {
    tree.visible_rows.iter().any(|row| row.node_id == node_id)
}

fn normalize_rel_path(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path.to_path_buf()
    }
}

fn watcher_plan_touches_rel_path(plan: &RefreshPlan, rel_path: &Path) -> bool {
    let rel_path = normalize_rel_path(rel_path);
    if rel_path == Path::new(".") {
        return !(plan.created_paths.is_empty()
            && plan.changed_paths.is_empty()
            && plan.removed_paths.is_empty());
    }

    plan.created_paths
        .iter()
        .chain(plan.changed_paths.iter())
        .chain(plan.removed_paths.iter())
        .any(|path| path == &rel_path || path.starts_with(&rel_path))
}

fn root_label_disambiguator(path: &Path, colliding_roots: &[PathBuf]) -> Option<String> {
    if colliding_roots.len() <= 1 {
        return None;
    }

    let target_parts = root_parent_parts(path);
    if target_parts.is_empty() {
        return Some(path.display().to_string());
    }

    for depth in 1..=target_parts.len() {
        let candidate = parent_suffix(&target_parts, depth);
        let is_unique = colliding_roots
            .iter()
            .filter(|other| other.as_path() != path)
            .all(|other| parent_suffix(&root_parent_parts(other), depth) != candidate);
        if is_unique {
            return Some(candidate);
        }
    }

    Some(target_parts.join("/"))
}

fn root_parent_parts(path: &Path) -> Vec<String> {
    path.parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn parent_suffix(parts: &[String], depth: usize) -> String {
    if parts.is_empty() {
        return String::new();
    }
    let depth = depth.min(parts.len());
    parts[parts.len() - depth..].join("/")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContentSearchStatus {
    #[default]
    Idle,
    Searching,
    Ready,
    Error,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContentSearchState {
    pub query: String,
    pub generation: SearchGeneration,
    pub active: bool,
    pub status: ContentSearchStatus,
    pub status_message: Option<String>,
    pub payload: SearchPayload,
    pub selected_hit_index: Option<usize>,
    #[serde(skip)]
    pub runtime: ContentSearchRuntime,
}

impl Default for ContentSearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            generation: SearchGeneration::default(),
            active: false,
            status: ContentSearchStatus::Idle,
            status_message: None,
            payload: SearchPayload::default(),
            selected_hit_index: None,
            runtime: ContentSearchRuntime::default(),
        }
    }
}

#[derive(Debug, Default)]
pub struct ContentSearchRuntime {
    pub worker: Option<ContentSearchWorker>,
}

#[derive(Debug, Clone)]
pub struct GitTabState {
    pub generation: GitGeneration,
    pub repo: Option<RepoHandle>,
    pub status_map: HashMap<PathBuf, GitPathStatus>,
    pub last_error: Option<String>,
    pub initialized: bool,
    pub needs_refresh: bool,
}

impl Default for GitTabState {
    fn default() -> Self {
        Self {
            generation: GitGeneration::default(),
            repo: None,
            status_map: HashMap::new(),
            last_error: None,
            initialized: false,
            needs_refresh: true,
        }
    }
}

impl GitTabState {
    pub fn repo_summary(&self) -> Option<GitRepoSummary> {
        let repo = self.repo.as_ref()?;
        Some(crate::git::backend::summarize_repo_statuses(
            repo,
            &self.status_map,
        ))
    }
}

fn fallback_tree(root: PathBuf, visibility: VisibilitySettings) -> TreeState {
    let mut tree = TreeState::new_for_root(root);
    tree.show_hidden = visibility.show_hidden;
    tree.respect_gitignore = visibility.respect_gitignore;
    tree
}

fn clamp_content_search_selection(current: Option<usize>, hit_count: usize) -> Option<usize> {
    if hit_count == 0 {
        return None;
    }

    Some(current.unwrap_or(0).min(hit_count - 1))
}

fn content_search_status_message(hit_count: usize) -> String {
    match hit_count {
        0 => "no results".to_string(),
        1 => "1 result".to_string(),
        count => format!("{count} results"),
    }
}

fn clamp_overlay_selection(current: usize, entry_count: usize) -> usize {
    if entry_count == 0 {
        0
    } else {
        current.min(entry_count - 1)
    }
}

fn move_overlay_selection(selected_index: &mut usize, entry_count: usize, delta: isize) -> bool {
    if entry_count == 0 {
        return false;
    }

    let next = if delta.is_negative() {
        selected_index.saturating_sub(delta.saturating_abs() as usize)
    } else {
        selected_index
            .saturating_add(delta as usize)
            .min(entry_count - 1)
    };
    if next == *selected_index {
        return false;
    }

    *selected_index = next;
    true
}
