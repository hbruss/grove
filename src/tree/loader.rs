use std::fs;
use std::path::{Path, PathBuf};

use crate::git::backend::GitStatus;

use super::model::{DirLoadState, Node, NodeId, NodeKind, TreeState, VisibilitySettings};

pub fn load_root_shallow(root: &Path) -> std::io::Result<TreeState> {
    load_root_shallow_with_visibility(root, VisibilitySettings::default())
}

pub fn load_root_shallow_with_visibility(
    root: &Path,
    visibility: VisibilitySettings,
) -> std::io::Result<TreeState> {
    let root_abs = fs::canonicalize(root)?;
    let mut tree = TreeState::new_for_root(root_abs.clone());
    tree.show_hidden = visibility.show_hidden;
    tree.respect_gitignore = visibility.respect_gitignore;

    let root_name = root_abs
        .file_name()
        .and_then(|s| s.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| root_abs.display().to_string());

    let root_node = Node {
        id: NodeId(0),
        parent: None,
        name: root_name,
        rel_path: PathBuf::from("."),
        kind: NodeKind::Directory,
        expanded: true,
        depth: 0,
        dir_load: DirLoadState::Unloaded,
        size: None,
        modified: None,
        git: GitStatus::Unmodified,
        is_hidden: false,
        selected: true,
        highlight_until: None,
        children: Vec::new(),
    };

    tree.path_to_id.insert(PathBuf::from("."), tree.root_id);
    tree.nodes[0] = Some(root_node);
    let root_id = tree.root_id;
    load_directory_children(&mut tree, root_id)?;
    tree.rebuild_visible_rows();
    Ok(tree)
}

pub fn expand_selected_directory(tree: &mut TreeState) -> std::io::Result<bool> {
    Ok(tree.expand_selected())
}

pub(crate) fn load_directory_children(tree: &mut TreeState, dir_id: NodeId) -> std::io::Result<()> {
    let (dir_abs, parent_depth, parent_kind, already_loaded, visibility) = {
        let parent = tree.node(dir_id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "directory node not found")
        })?;
        (
            tree.root_abs.join(&parent.rel_path),
            parent.depth,
            parent.kind.clone(),
            matches!(parent.dir_load, DirLoadState::Loaded),
            tree.visibility_settings(),
        )
    };

    if !matches!(
        parent_kind,
        NodeKind::Directory | NodeKind::SymlinkDirectory
    ) {
        return Ok(());
    }

    if already_loaded {
        return Ok(());
    }

    let children = visible_entries(&dir_abs, &tree.root_abs, visibility)?;
    let mut child_ids = Vec::with_capacity(children.len());

    for entry in children {
        let child_rel_path = match entry.path_abs.strip_prefix(&tree.root_abs) {
            Ok(rel_path) => rel_path.to_path_buf(),
            Err(_) => continue,
        };

        if let Some(existing_id) = tree.path_to_id.get(&child_rel_path).copied() {
            child_ids.push(existing_id);
            continue;
        }

        let child_id = NodeId(tree.nodes.len() as u32);
        let child = build_child_node(
            child_id,
            dir_id,
            parent_depth.saturating_add(1),
            entry.name,
            child_rel_path.clone(),
            entry.kind,
        );

        child_ids.push(child_id);
        tree.path_to_id.insert(child_rel_path, child_id);
        tree.nodes.push(Some(child));
    }

    if let Some(parent) = tree.node_mut(dir_id) {
        parent.children = child_ids;
        parent.dir_load = DirLoadState::Loaded;
    }

    Ok(())
}

struct LoadedEntry {
    path_abs: PathBuf,
    name: String,
    kind: NodeKind,
}

fn visible_entries(
    dir_abs: &Path,
    root_abs: &Path,
    visibility: VisibilitySettings,
) -> std::io::Result<Vec<LoadedEntry>> {
    if !visibility.respect_gitignore {
        return raw_visible_entries(dir_abs, visibility);
    }

    let mut entries = Vec::new();
    let walker = super::walk::build_directory_walker(dir_abs, root_abs, visibility);
    for result in walker.build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path_abs = entry.path();
        if path_abs == dir_abs || path_abs.parent() != Some(dir_abs) {
            continue;
        }

        let Some(file_name) = path_abs.file_name() else {
            continue;
        };
        let kind = match classify_path_kind(path_abs) {
            Ok(kind) => kind,
            Err(_) => continue,
        };
        entries.push(LoadedEntry {
            path_abs: path_abs.to_path_buf(),
            name: file_name.to_string_lossy().to_string(),
            kind,
        });
    }
    Ok(entries)
}

fn raw_visible_entries(
    dir_abs: &Path,
    visibility: VisibilitySettings,
) -> std::io::Result<Vec<LoadedEntry>> {
    let mut entries = Vec::new();
    for path_abs in sorted_entries(dir_abs)? {
        let Some(file_name) = path_abs.file_name() else {
            continue;
        };
        let name = file_name.to_string_lossy().to_string();
        if !visibility.show_hidden && name.starts_with('.') {
            continue;
        }
        let kind = match classify_path_kind(&path_abs) {
            Ok(kind) => kind,
            Err(_) => continue,
        };
        entries.push(LoadedEntry {
            path_abs,
            name,
            kind,
        });
    }
    Ok(entries)
}

fn classify_path_kind(path: &Path) -> std::io::Result<NodeKind> {
    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_symlink() {
        return match fs::metadata(path) {
            Ok(meta) => {
                if meta.is_dir() {
                    Ok(NodeKind::SymlinkDirectory)
                } else {
                    Ok(NodeKind::SymlinkFile)
                }
            }
            Err(_) => Ok(NodeKind::SymlinkFile),
        };
    }

    if file_type.is_dir() {
        Ok(NodeKind::Directory)
    } else {
        Ok(NodeKind::File)
    }
}

fn sorted_entries(dir_abs: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut children = fs::read_dir(dir_abs)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    children.sort_by_key(|path| {
        path.file_name()
            .map(|file_name| file_name.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    Ok(children)
}

fn build_child_node(
    child_id: NodeId,
    parent_id: NodeId,
    depth: u16,
    name: String,
    rel_path: PathBuf,
    kind: NodeKind,
) -> Node {
    let is_directory = matches!(kind, NodeKind::Directory | NodeKind::SymlinkDirectory);
    let is_hidden = name.starts_with('.');
    Node {
        id: child_id,
        parent: Some(parent_id),
        name,
        rel_path,
        kind,
        expanded: false,
        depth,
        dir_load: if is_directory {
            DirLoadState::Unloaded
        } else {
            DirLoadState::Loaded
        },
        size: None,
        modified: None,
        git: GitStatus::Unmodified,
        is_hidden,
        selected: false,
        highlight_until: None,
        children: Vec::new(),
    }
}
