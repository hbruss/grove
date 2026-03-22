use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use grove::tree::loader::{load_root_shallow, load_root_shallow_with_visibility};
use grove::tree::model::VisibilitySettings;
#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn loads_root_and_immediate_children_only() {
    let root = make_temp_dir("grove-tree-model");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("beta.txt"), "hello").expect("should create beta file");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");

    let tree = load_root_shallow(&root).expect("shallow loader should succeed");

    assert_eq!(tree.root_id.0, 0);
    assert_eq!(
        tree.visible_rows.len(),
        3,
        "expected root + 2 immediate children"
    );
    assert_eq!(
        tree.node(tree.root_id)
            .expect("root should exist")
            .children
            .len(),
        2
    );
    assert!(
        !tree
            .path_to_id
            .contains_key(&PathBuf::from("alpha/nested.txt")),
        "nested entries must not be loaded in shallow mode"
    );

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
fn selection_stays_in_bounds_for_visible_rows() {
    let root = make_temp_dir("grove-tree-selection");
    fs::write(root.join("one.txt"), "1").expect("should create file");
    fs::write(root.join("two.txt"), "2").expect("should create file");

    let mut tree = load_root_shallow(&root).expect("shallow loader should succeed");
    assert_eq!(tree.selected_row, 0);

    tree.select_prev();
    assert_eq!(tree.selected_row, 0);

    tree.select_next();
    assert_eq!(tree.selected_row, 1);
    tree.select_next();
    assert_eq!(tree.selected_row, 2);
    tree.select_next();
    assert_eq!(tree.selected_row, 2);

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
#[cfg(unix)]
fn broken_symlink_does_not_fail_shallow_load() {
    let root = make_temp_dir("grove-tree-broken-symlink");
    fs::write(root.join("ok.txt"), "ok").expect("should create normal file");
    symlink(root.join("missing-target"), root.join("broken-link")).expect("should create symlink");

    let tree = load_root_shallow(&root).expect("loader should tolerate broken symlink entries");

    assert!(
        tree.path_to_id.contains_key(&PathBuf::from("ok.txt")),
        "normal entries should still load when a sibling symlink is broken"
    );
    assert!(
        tree.path_to_id.contains_key(&PathBuf::from("broken-link")),
        "broken symlink should not abort loading"
    );

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
fn node_selected_flag_tracks_selected_row() {
    let root = make_temp_dir("grove-tree-selected-sync");
    fs::write(root.join("one.txt"), "1").expect("should create file");
    fs::write(root.join("two.txt"), "2").expect("should create file");

    let mut tree = load_root_shallow(&root).expect("shallow loader should succeed");
    assert_selected_consistent(&tree);

    tree.select_next();
    assert_selected_consistent(&tree);

    tree.select_next();
    assert_selected_consistent(&tree);

    tree.select_prev();
    assert_selected_consistent(&tree);

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
fn expanding_directory_loads_immediate_children_and_updates_visible_rows() {
    let root = make_temp_dir("grove-tree-expand");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");

    let mut tree = load_root_shallow(&root).expect("shallow loader should succeed");
    tree.select_next();

    assert!(tree.expand_selected(), "directory should expand");
    assert!(
        tree.path_to_id
            .contains_key(&PathBuf::from("alpha/nested.txt")),
        "expanding should load immediate children for the selected directory"
    );
    assert_eq!(tree.visible_rows.len(), 3, "root, alpha, nested");

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
fn expanding_directory_with_gitignore_respects_local_rules_and_stays_shallow() {
    let root = make_temp_dir("grove-tree-expand-gitignore");
    fs::create_dir_all(root.join(".git")).expect("should create pseudo git dir");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::create_dir_all(root.join("alpha").join("visible-dir"))
        .expect("should create visible child dir");
    fs::create_dir_all(root.join("alpha").join("ignored-dir"))
        .expect("should create ignored child dir");
    fs::write(
        root.join("alpha").join(".gitignore"),
        "ignored-dir/\nignored.txt\n",
    )
    .expect("should create nested gitignore");
    fs::write(root.join("alpha").join("visible.txt"), "visible")
        .expect("should create visible file");
    fs::write(root.join("alpha").join("ignored.txt"), "ignored")
        .expect("should create ignored file");
    fs::write(
        root.join("alpha").join("visible-dir").join("nested.txt"),
        "nested",
    )
    .expect("should create nested file");

    let mut tree = load_root_shallow_with_visibility(
        &root,
        VisibilitySettings {
            show_hidden: false,
            respect_gitignore: true,
        },
    )
    .expect("shallow loader should succeed");
    tree.select_next();

    assert!(tree.expand_selected(), "directory should expand");
    assert!(
        tree.path_to_id
            .contains_key(&PathBuf::from("alpha/visible.txt")),
        "visible files should load for gitignore-aware expansion"
    );
    assert!(
        tree.path_to_id
            .contains_key(&PathBuf::from("alpha/visible-dir")),
        "visible child directories should load for gitignore-aware expansion"
    );
    assert!(
        !tree
            .path_to_id
            .contains_key(&PathBuf::from("alpha/ignored.txt")),
        "ignored files should stay hidden during gitignore-aware expansion"
    );
    assert!(
        !tree
            .path_to_id
            .contains_key(&PathBuf::from("alpha/ignored-dir")),
        "ignored child directories should stay hidden during gitignore-aware expansion"
    );
    assert!(
        !tree
            .path_to_id
            .contains_key(&PathBuf::from("alpha/visible-dir/nested.txt")),
        "gitignore-aware expansion should stay shallow and avoid loading grandchildren"
    );

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
fn collapsing_directory_hides_descendants_without_reloading_them() {
    let root = make_temp_dir("grove-tree-collapse");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");

    let mut tree = load_root_shallow(&root).expect("shallow loader should succeed");
    tree.select_next();
    assert!(tree.expand_selected(), "directory should expand");

    let nodes_after_first_expand = tree.nodes.len();
    assert!(
        tree.collapse_selected_or_select_parent(),
        "expanded directory should collapse"
    );
    assert_eq!(
        tree.visible_rows.len(),
        2,
        "root and collapsed directory only"
    );

    assert!(tree.expand_selected(), "directory should expand again");
    assert_eq!(
        tree.nodes.len(),
        nodes_after_first_expand,
        "re-expanding a previously loaded directory must not duplicate nodes"
    );

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
fn left_on_child_selects_parent_directory() {
    let root = make_temp_dir("grove-tree-parent-select");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");

    let mut tree = load_root_shallow(&root).expect("shallow loader should succeed");
    tree.select_next();
    assert!(tree.expand_selected(), "directory should expand");

    tree.select_next();
    assert!(
        tree.collapse_selected_or_select_parent(),
        "left on a child row should move selection to its parent"
    );

    let selected_row = tree
        .visible_rows
        .get(tree.selected_row)
        .expect("selected row should exist");
    let selected_node = tree
        .node(selected_row.node_id)
        .expect("selected node should exist");
    assert_eq!(selected_node.rel_path, PathBuf::from("alpha"));

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

#[test]
fn ensure_selected_row_visible_updates_scroll_row_for_small_viewport() {
    let root = make_temp_dir("grove-tree-scroll");
    for idx in 0..8_u8 {
        fs::write(root.join(format!("file-{idx:03}.txt")), "x").expect("should create file");
    }

    let mut tree = load_root_shallow(&root).expect("shallow loader should succeed");
    for _ in 0..6 {
        tree.select_next();
    }

    assert!(
        tree.ensure_selected_row_visible(3),
        "scroll row should move to keep the selected row visible"
    );
    assert!(tree.scroll_row > 0, "scroll row should advance");
    assert!(
        tree.selected_row < tree.scroll_row + 3,
        "selected row should be within viewport after adjustment"
    );

    fs::remove_dir_all(&root).expect("temp root should be removed");
}

fn make_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}"));
    fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

fn assert_selected_consistent(tree: &grove::tree::model::TreeState) {
    let selected_node_ids = tree
        .nodes
        .iter()
        .filter_map(|node| node.as_ref())
        .filter(|node| node.selected)
        .map(|node| node.id)
        .collect::<Vec<_>>();

    assert_eq!(
        selected_node_ids.len(),
        1,
        "expected exactly one selected node flag"
    );

    let selected_row = tree
        .visible_rows
        .get(tree.selected_row)
        .expect("selected row should exist");

    assert_eq!(selected_node_ids[0], selected_row.node_id);
}
