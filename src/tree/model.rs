use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::config::SortMode;
use crate::git::backend::GitStatus;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Directory,
    SymlinkFile,
    SymlinkDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirLoadState {
    Unloaded,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub name: String,
    pub rel_path: PathBuf,
    pub kind: NodeKind,
    pub expanded: bool,
    pub depth: u16,
    pub dir_load: DirLoadState,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub git: GitStatus,
    pub is_hidden: bool,
    pub selected: bool,
    pub highlight_until: Option<Instant>,
    pub children: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VisibleRow {
    pub node_id: NodeId,
    pub depth: u16,
    pub is_match: bool,
    pub match_ranges: Vec<std::ops::Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexState {
    Idle,
    Indexing { indexed_paths: usize },
    Complete,
    Error(String),
}

impl Default for IndexState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct VisibilitySettings {
    pub show_hidden: bool,
    pub respect_gitignore: bool,
}

impl Default for VisibilitySettings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            respect_gitignore: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeState {
    pub root_abs: PathBuf,
    pub nodes: Vec<Option<Node>>,
    pub root_id: NodeId,
    pub path_to_id: HashMap<PathBuf, NodeId>,
    pub base_visible_rows: Vec<VisibleRow>,
    pub visible_rows: Vec<VisibleRow>,
    pub selected_row: usize,
    pub scroll_row: usize,
    pub pre_filter_selected: Option<NodeId>,
    pub pre_filter_scroll_row: Option<usize>,
    pub multiselect: BTreeSet<NodeId>,
    pub sort_mode: SortMode,
    pub show_hidden: bool,
    pub respect_gitignore: bool,
    pub index_state: IndexState,
}

impl Default for TreeState {
    fn default() -> Self {
        Self {
            root_abs: PathBuf::from("."),
            nodes: Vec::new(),
            root_id: NodeId(0),
            path_to_id: HashMap::new(),
            base_visible_rows: Vec::new(),
            visible_rows: Vec::new(),
            selected_row: 0,
            scroll_row: 0,
            pre_filter_selected: None,
            pre_filter_scroll_row: None,
            multiselect: BTreeSet::new(),
            sort_mode: SortMode::Name,
            show_hidden: false,
            respect_gitignore: true,
            index_state: IndexState::Idle,
        }
    }
}

impl TreeState {
    pub fn new_for_root(root_abs: PathBuf) -> Self {
        let mut tree = Self {
            root_abs,
            ..Self::default()
        };
        tree.nodes.push(None);
        tree
    }

    pub fn rebuild_visible_rows(&mut self) {
        let selected_node_id = self.selected_node_id();
        let mut rows = Vec::new();
        self.collect_visible_rows(self.root_id, 0, &mut rows);
        self.base_visible_rows = rows.clone();
        self.visible_rows = rows;
        if self.scroll_row >= self.visible_rows.len() {
            self.scroll_row = self.visible_rows.len().saturating_sub(1);
        }
        if !self.restore_selected_node(selected_node_id) {
            self.sync_selection_flags();
        }
    }

    fn collect_visible_rows(&self, node_id: NodeId, depth: u16, rows: &mut Vec<VisibleRow>) {
        rows.push(VisibleRow {
            node_id,
            depth,
            is_match: false,
            match_ranges: Vec::new(),
        });

        let maybe_children = self
            .node(node_id)
            .filter(|node| node.expanded)
            .map(|node| node.children.clone());

        if let Some(children) = maybe_children {
            for child_id in children {
                self.collect_visible_rows(child_id, depth.saturating_add(1), rows);
            }
        }
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize).and_then(Option::as_ref)
    }

    pub fn select_prev(&mut self) {
        if self.selected_row > 0 {
            self.selected_row -= 1;
        }
        self.sync_selection_flags();
    }

    pub fn select_next(&mut self) {
        if self.selected_row + 1 < self.visible_rows.len() {
            self.selected_row += 1;
        }
        self.sync_selection_flags();
    }

    pub fn ensure_selected_row_visible(&mut self, viewport_height: usize) -> bool {
        if self.visible_rows.is_empty() {
            self.scroll_row = 0;
            return false;
        }

        let max_scroll = if viewport_height == 0 {
            0
        } else {
            self.visible_rows.len().saturating_sub(viewport_height)
        };
        let mut new_scroll_row = self.scroll_row.min(max_scroll);

        if viewport_height == 0 {
            if new_scroll_row == self.scroll_row {
                return false;
            }
            self.scroll_row = new_scroll_row;
            return true;
        }

        if self.selected_row < new_scroll_row {
            new_scroll_row = self.selected_row;
        } else {
            let viewport_end = new_scroll_row + viewport_height;
            if self.selected_row >= viewport_end {
                new_scroll_row = self.selected_row + 1 - viewport_height;
            } else {
                new_scroll_row = new_scroll_row.min(max_scroll);
            }
        }

        if new_scroll_row == self.scroll_row {
            return false;
        }

        self.scroll_row = new_scroll_row;
        true
    }

    pub fn expand_selected(&mut self) -> bool {
        let Some(selected_node_id) = self.selected_node_id() else {
            return false;
        };

        self.expand_node(selected_node_id)
    }

    pub fn expand_rel_path(&mut self, rel_path: &Path) -> bool {
        let Some(node_id) = self.path_to_id.get(rel_path).copied() else {
            return false;
        };
        self.expand_node(node_id)
    }

    pub fn expanded_directory_paths(&self) -> Vec<PathBuf> {
        let mut paths = self
            .nodes
            .iter()
            .filter_map(Option::as_ref)
            .filter(|node| {
                node.id != self.root_id && node.expanded && is_directory_kind(&node.kind)
            })
            .map(|node| node.rel_path.clone())
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| path.components().count());
        paths
    }

    fn expand_node(&mut self, node_id: NodeId) -> bool {
        let Some((kind, expanded, dir_load)) = self
            .node(node_id)
            .map(|node| (node.kind.clone(), node.expanded, node.dir_load.clone()))
        else {
            return false;
        };

        if !is_directory_kind(&kind) || expanded {
            return false;
        }

        let should_load = matches!(dir_load, DirLoadState::Unloaded | DirLoadState::Error(_));
        if should_load && let Err(err) = crate::tree::loader::load_directory_children(self, node_id)
        {
            if let Some(node) = self.node_mut(node_id) {
                node.dir_load = DirLoadState::Error(err.to_string());
            }
            return false;
        }

        if let Some(node) = self.node_mut(node_id) {
            node.expanded = true;
        }
        self.rebuild_visible_rows();
        true
    }

    pub fn collapse_selected_or_select_parent(&mut self) -> bool {
        let Some(selected_node_id) = self.selected_node_id() else {
            return false;
        };

        let Some((kind, expanded, parent)) = self
            .node(selected_node_id)
            .map(|node| (node.kind.clone(), node.expanded, node.parent))
        else {
            return false;
        };

        if selected_node_id != self.root_id && is_directory_kind(&kind) && expanded {
            if let Some(node) = self.node_mut(selected_node_id) {
                node.expanded = false;
            }
            self.rebuild_visible_rows();
            return true;
        }

        match parent {
            Some(parent_id) => self.select_node(parent_id),
            None => false,
        }
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0 as usize).and_then(Option::as_mut)
    }

    pub fn visibility_settings(&self) -> VisibilitySettings {
        VisibilitySettings {
            show_hidden: self.show_hidden,
            respect_gitignore: self.respect_gitignore,
        }
    }

    pub fn selected_rel_path(&self) -> Option<PathBuf> {
        self.selected_node_id()
            .and_then(|node_id| self.node(node_id).map(|node| node.rel_path.clone()))
    }

    pub fn select_rel_path(&mut self, rel_path: &Path) -> bool {
        let Some(node_id) = self.path_to_id.get(rel_path).copied() else {
            return false;
        };
        self.select_node(node_id)
    }

    pub fn capture_pre_filter_state(&mut self) {
        self.pre_filter_selected = self.selected_node_id();
        self.pre_filter_scroll_row = Some(self.scroll_row);
    }

    pub fn restore_pre_filter_state(&mut self) {
        self.visible_rows = self.base_visible_rows.clone();

        if let Some(node_id) = self.pre_filter_selected.take() {
            let _ = self.select_node(node_id);
        } else {
            self.sync_selection_flags();
        }

        let max_scroll = self.visible_rows.len().saturating_sub(1);
        self.scroll_row = self
            .pre_filter_scroll_row
            .take()
            .unwrap_or_default()
            .min(max_scroll);
    }

    pub fn apply_filtered_matches(&mut self, matched_node_ids: &[NodeId]) {
        let selected_node_id = self.selected_node_id();
        let match_set = matched_node_ids.iter().copied().collect::<HashSet<_>>();
        let allowed = crate::tree::filter::collect_ancestor_ids(self, matched_node_ids);
        let mut rows = Vec::new();
        self.collect_filtered_rows(self.root_id, 0, &allowed, &match_set, &mut rows);
        if rows.is_empty() {
            rows.push(VisibleRow {
                node_id: self.root_id,
                depth: 0,
                is_match: false,
                match_ranges: Vec::new(),
            });
        }
        self.visible_rows = rows;
        self.scroll_row = 0;

        if self.restore_selected_node(selected_node_id) {
            return;
        }

        if let Some(first_match) = matched_node_ids.first().copied() {
            let _ = self.select_node(first_match);
        } else {
            self.selected_row = 0;
            self.sync_selection_flags();
        }
    }

    fn collect_filtered_rows(
        &self,
        node_id: NodeId,
        depth: u16,
        allowed: &HashSet<NodeId>,
        match_set: &HashSet<NodeId>,
        rows: &mut Vec<VisibleRow>,
    ) {
        if !allowed.contains(&node_id) {
            return;
        }

        rows.push(VisibleRow {
            node_id,
            depth,
            is_match: match_set.contains(&node_id),
            match_ranges: Vec::new(),
        });

        if let Some(children) = self.node(node_id).map(|node| node.children.clone()) {
            for child_id in children {
                self.collect_filtered_rows(
                    child_id,
                    depth.saturating_add(1),
                    allowed,
                    match_set,
                    rows,
                );
            }
        }
    }

    fn sync_selection_flags(&mut self) {
        let visible_len = self.visible_rows.len();
        if visible_len == 0 {
            for node in self.nodes.iter_mut().filter_map(Option::as_mut) {
                node.selected = false;
            }
            self.selected_row = 0;
            return;
        }

        if self.selected_row >= visible_len {
            self.selected_row = visible_len - 1;
        }

        let selected_node_id = self.visible_rows[self.selected_row].node_id;
        for node in self.nodes.iter_mut().filter_map(Option::as_mut) {
            node.selected = node.id == selected_node_id;
        }
    }

    fn selected_node_id(&self) -> Option<NodeId> {
        self.visible_rows
            .get(self.selected_row)
            .map(|row| row.node_id)
    }

    fn select_node(&mut self, node_id: NodeId) -> bool {
        let Some(row_index) = self
            .visible_rows
            .iter()
            .position(|row| row.node_id == node_id)
        else {
            return false;
        };

        if self.selected_row == row_index {
            self.sync_selection_flags();
            return false;
        }

        self.selected_row = row_index;
        self.sync_selection_flags();
        true
    }

    fn restore_selected_node(&mut self, node_id: Option<NodeId>) -> bool {
        let Some(node_id) = node_id else {
            return false;
        };
        let Some(row_index) = self
            .visible_rows
            .iter()
            .position(|row| row.node_id == node_id)
        else {
            return false;
        };
        self.selected_row = row_index;
        self.sync_selection_flags();
        true
    }
}

fn is_directory_kind(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Directory | NodeKind::SymlinkDirectory)
}
