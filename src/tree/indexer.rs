use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

use crate::debug_log;

use super::model::{DirLoadState, Node, NodeId, NodeKind, TreeState, VisibilitySettings};

const INDEX_BATCH_SIZE: usize = 128;

#[derive(Debug, Clone)]
pub struct PathIndexEntry {
    pub rel_path: PathBuf,
    pub name: String,
    pub kind: NodeKind,
    pub utf32_path: Utf32String,
}

#[derive(Debug, Clone, Default)]
pub struct PathIndexSnapshot {
    pub entries: Vec<PathIndexEntry>,
}

#[derive(Debug)]
pub enum PathIndexEvent {
    Batch(Vec<PathIndexEntry>),
    Complete,
    Error(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PathIndexStreamOutcome {
    indexed_paths: usize,
    receiver_dropped: bool,
}

pub fn start_background_index(root_abs: PathBuf) -> Receiver<PathIndexEvent> {
    start_background_index_with_visibility(root_abs, VisibilitySettings::default())
}

pub fn start_background_index_with_visibility(
    root_abs: PathBuf,
    visibility: VisibilitySettings,
) -> Receiver<PathIndexEvent> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let started_at = Instant::now();
        debug_log::log(&format!(
            "component=path_index_worker phase=start show_hidden={} respect_gitignore={} root={}",
            visibility.show_hidden,
            visibility.respect_gitignore,
            root_abs.display()
        ));
        let mut batch = Vec::with_capacity(INDEX_BATCH_SIZE);
        let outcome = match walk_directory_stream(&root_abs, visibility, &sender, &mut batch) {
            Ok(outcome) => outcome,
            Err(err) => {
                let _ = sender.send(PathIndexEvent::Error(err.to_string()));
                debug_log::log(&format!(
                    "component=path_index_worker phase=error indexed_paths=0 duration_ms={} error={}",
                    started_at.elapsed().as_millis(),
                    err
                ));
                return;
            }
        };
        if outcome.receiver_dropped {
            debug_log::log(&format!(
                "component=path_index_worker phase=cancelled indexed_paths={} duration_ms={} reason=receiver_dropped",
                outcome.indexed_paths,
                started_at.elapsed().as_millis()
            ));
            return;
        }
        if !batch.is_empty() && sender.send(PathIndexEvent::Batch(batch)).is_err() {
            debug_log::log(&format!(
                "component=path_index_worker phase=cancelled indexed_paths={} duration_ms={} reason=receiver_dropped",
                outcome.indexed_paths,
                started_at.elapsed().as_millis()
            ));
            return;
        }
        let _ = sender.send(PathIndexEvent::Complete);
        debug_log::log(&format!(
            "component=path_index_worker phase=complete indexed_paths={} duration_ms={}",
            outcome.indexed_paths,
            started_at.elapsed().as_millis()
        ));
    });
    receiver
}

pub fn build_snapshot(root_abs: &Path) -> std::io::Result<PathIndexSnapshot> {
    build_snapshot_with_visibility(root_abs, VisibilitySettings::default())
}

pub fn build_snapshot_with_visibility(
    root_abs: &Path,
    visibility: VisibilitySettings,
) -> std::io::Result<PathIndexSnapshot> {
    let root_abs = std::fs::canonicalize(root_abs)?;
    if !visibility.respect_gitignore {
        let mut entries = Vec::new();
        walk_directory_collect_raw(&root_abs, Path::new("."), visibility, &mut entries)?;
        return Ok(PathIndexSnapshot { entries });
    }

    let mut entries = Vec::new();
    let walker = super::walk::build_recursive_walker(&root_abs, visibility);
    for result in walker.build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let Some(index_entry) = path_index_entry(&root_abs, entry.path())? else {
            continue;
        };
        entries.push(index_entry);
    }
    Ok(PathIndexSnapshot { entries })
}

pub fn merge_snapshot_into_tree(tree: &mut TreeState, snapshot: &PathIndexSnapshot) {
    for entry in &snapshot.entries {
        ensure_entry_in_tree(tree, entry);
    }
}

pub fn merge_entries_into_tree(tree: &mut TreeState, entries: &[PathIndexEntry]) {
    for entry in entries {
        ensure_entry_in_tree(tree, entry);
    }
}

pub fn rank_matches(snapshot: &PathIndexSnapshot, query: &str) -> Vec<PathBuf> {
    if query.is_empty() {
        return Vec::new();
    }

    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut matches = snapshot
        .entries
        .iter()
        .filter_map(|entry| {
            pattern
                .score(entry.utf32_path.slice(..), &mut matcher)
                .map(|score| (entry.rel_path.clone(), score))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    matches.into_iter().map(|(rel_path, _)| rel_path).collect()
}

fn walk_directory_stream(
    root_abs: &Path,
    visibility: VisibilitySettings,
    sender: &mpsc::Sender<PathIndexEvent>,
    batch: &mut Vec<PathIndexEntry>,
) -> std::io::Result<PathIndexStreamOutcome> {
    if !visibility.respect_gitignore {
        return walk_directory_stream_raw(root_abs, Path::new("."), visibility, sender, batch);
    }

    let walker = super::walk::build_recursive_walker(root_abs, visibility);
    let mut indexed_paths = 0usize;
    for result in walker.build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let Some(index_entry) = path_index_entry(root_abs, entry.path())? else {
            continue;
        };

        batch.push(index_entry);
        indexed_paths = indexed_paths.saturating_add(1);

        if batch.len() >= INDEX_BATCH_SIZE {
            let flushed = std::mem::take(batch);
            if sender.send(PathIndexEvent::Batch(flushed)).is_err() {
                return Ok(PathIndexStreamOutcome {
                    indexed_paths,
                    receiver_dropped: true,
                });
            }
        }
    }

    Ok(PathIndexStreamOutcome {
        indexed_paths,
        receiver_dropped: false,
    })
}

fn walk_directory_collect_raw(
    dir_abs: &Path,
    parent_rel_path: &Path,
    visibility: VisibilitySettings,
    entries: &mut Vec<PathIndexEntry>,
) -> std::io::Result<()> {
    for path_abs in sorted_entries(dir_abs)? {
        let Some(file_name) = path_abs.file_name() else {
            continue;
        };
        let child_name = file_name.to_string_lossy().to_string();
        if !visibility.show_hidden && child_name.starts_with('.') {
            continue;
        }

        let child_rel_path = child_rel_path(parent_rel_path, file_name);
        let child_kind = match classify_path_kind(&path_abs) {
            Ok(kind) => kind,
            Err(_) => continue,
        };

        entries.push(PathIndexEntry {
            utf32_path: Utf32String::from(child_rel_path.to_string_lossy().to_string()),
            rel_path: child_rel_path.clone(),
            name: child_name,
            kind: child_kind.clone(),
        });

        if matches!(child_kind, NodeKind::Directory) {
            walk_directory_collect_raw(&path_abs, &child_rel_path, visibility, entries)?;
        }
    }

    Ok(())
}

fn walk_directory_stream_raw(
    dir_abs: &Path,
    parent_rel_path: &Path,
    visibility: VisibilitySettings,
    sender: &mpsc::Sender<PathIndexEvent>,
    batch: &mut Vec<PathIndexEntry>,
) -> std::io::Result<PathIndexStreamOutcome> {
    let mut indexed_paths = 0usize;
    for path_abs in sorted_entries(dir_abs)? {
        let Some(file_name) = path_abs.file_name() else {
            continue;
        };
        let child_name = file_name.to_string_lossy().to_string();
        if !visibility.show_hidden && child_name.starts_with('.') {
            continue;
        }

        let child_rel_path = child_rel_path(parent_rel_path, file_name);
        let child_kind = match classify_path_kind(&path_abs) {
            Ok(kind) => kind,
            Err(_) => continue,
        };

        batch.push(PathIndexEntry {
            utf32_path: Utf32String::from(child_rel_path.to_string_lossy().to_string()),
            rel_path: child_rel_path.clone(),
            name: child_name,
            kind: child_kind.clone(),
        });
        indexed_paths = indexed_paths.saturating_add(1);

        if batch.len() >= INDEX_BATCH_SIZE {
            let flushed = std::mem::take(batch);
            if sender.send(PathIndexEvent::Batch(flushed)).is_err() {
                return Ok(PathIndexStreamOutcome {
                    indexed_paths,
                    receiver_dropped: true,
                });
            }
        }

        if matches!(child_kind, NodeKind::Directory) {
            let outcome =
                walk_directory_stream_raw(&path_abs, &child_rel_path, visibility, sender, batch)?;
            indexed_paths = indexed_paths.saturating_add(outcome.indexed_paths);
            if outcome.receiver_dropped {
                return Ok(PathIndexStreamOutcome {
                    indexed_paths,
                    receiver_dropped: true,
                });
            }
        }
    }

    Ok(PathIndexStreamOutcome {
        indexed_paths,
        receiver_dropped: false,
    })
}

fn path_index_entry(root_abs: &Path, path_abs: &Path) -> std::io::Result<Option<PathIndexEntry>> {
    let rel_path = match path_abs.strip_prefix(root_abs) {
        Ok(rel_path) if !rel_path.as_os_str().is_empty() => rel_path.to_path_buf(),
        _ => return Ok(None),
    };
    let Some(file_name) = path_abs.file_name() else {
        return Ok(None);
    };
    let kind = classify_path_kind(path_abs)?;
    let name = file_name.to_string_lossy().to_string();
    Ok(Some(PathIndexEntry {
        utf32_path: Utf32String::from(rel_path.to_string_lossy().to_string()),
        rel_path,
        name,
        kind,
    }))
}

fn ensure_entry_in_tree(tree: &mut TreeState, entry: &PathIndexEntry) {
    if tree.path_to_id.contains_key(&entry.rel_path) {
        return;
    }

    let parent_rel_path = parent_rel_path(&entry.rel_path);
    let Some(parent_id) = tree.path_to_id.get(&parent_rel_path).copied() else {
        return;
    };

    let child_id = NodeId(tree.nodes.len() as u32);
    let depth = parent_depth(tree, parent_id).saturating_add(1);
    let dir_load = if matches!(entry.kind, NodeKind::Directory | NodeKind::SymlinkDirectory) {
        DirLoadState::Unloaded
    } else {
        DirLoadState::Loaded
    };
    let node = Node {
        id: child_id,
        parent: Some(parent_id),
        name: entry.name.clone(),
        rel_path: entry.rel_path.clone(),
        kind: entry.kind.clone(),
        expanded: false,
        depth,
        dir_load,
        size: None,
        modified: None,
        git: crate::git::backend::GitStatus::Unmodified,
        is_hidden: entry.name.starts_with('.'),
        selected: false,
        highlight_until: None,
        children: Vec::new(),
    };

    tree.path_to_id.insert(entry.rel_path.clone(), child_id);
    tree.nodes.push(Some(node));
    if let Some(parent) = tree.node_mut(parent_id) {
        if !parent.children.contains(&child_id) {
            parent.children.push(child_id);
        }
        parent.dir_load = DirLoadState::Loaded;
    }
}

fn parent_depth(tree: &TreeState, parent_id: NodeId) -> u16 {
    tree.node(parent_id)
        .map(|node| node.depth)
        .unwrap_or_default()
}

fn parent_rel_path(rel_path: &Path) -> PathBuf {
    rel_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn sorted_entries(dir_abs: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut children = std::fs::read_dir(dir_abs)?
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

fn child_rel_path(parent_rel_path: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    if parent_rel_path == Path::new(".") {
        PathBuf::from(file_name)
    } else {
        parent_rel_path.join(file_name)
    }
}

fn classify_path_kind(path: &Path) -> std::io::Result<NodeKind> {
    let file_type = std::fs::symlink_metadata(path)?.file_type();
    if file_type.is_symlink() {
        return match std::fs::metadata(path) {
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

#[cfg(test)]
mod tests {
    use super::*;

    use git2::Repository;
    use std::fs;
    use std::sync::mpsc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn raw_stream_stops_when_receiver_is_dropped() {
        let root = make_temp_dir("grove-indexer-cancel");
        for index in 0..300 {
            fs::write(root.join(format!("file-{index:03}.txt")), "data")
                .expect("file should be written");
        }

        let (sender, receiver) = mpsc::channel();
        drop(receiver);

        let mut batch = Vec::with_capacity(INDEX_BATCH_SIZE);
        walk_directory_stream_raw(
            &root,
            Path::new("."),
            VisibilitySettings {
                show_hidden: true,
                respect_gitignore: false,
            },
            &sender,
            &mut batch,
        )
        .expect("raw stream should return cleanly when cancelled");

        assert!(
            batch.is_empty(),
            "disconnected receivers should stop raw indexing before more work accumulates"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn gitignore_stream_stops_when_receiver_is_dropped() {
        let root = make_temp_dir("grove-indexer-gitignore-cancel");
        Repository::init(&root).expect("repo should initialize");
        for index in 0..300 {
            fs::write(root.join(format!("file-{index:03}.txt")), "data")
                .expect("file should be written");
        }

        let (sender, receiver) = mpsc::channel();
        drop(receiver);

        let mut batch = Vec::with_capacity(INDEX_BATCH_SIZE);
        walk_directory_stream(
            &root,
            VisibilitySettings {
                show_hidden: true,
                respect_gitignore: true,
            },
            &sender,
            &mut batch,
        )
        .expect("gitignore stream should return cleanly when cancelled");

        assert!(
            batch.is_empty(),
            "disconnected receivers should stop gitignore-aware indexing before more work accumulates"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    fn make_temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{label}-{unique}"));
        fs::create_dir_all(&root).expect("temp root should be created");
        root
    }
}
