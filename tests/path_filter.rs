use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use grove::app::{App, PathIndexState, PathIndexStatus, TabState};
use grove::tree::indexer::{PathIndexEvent, build_snapshot, build_snapshot_with_visibility};
use grove::tree::model::VisibilitySettings;
use grove::watcher::RefreshPlan;

#[test]
fn filtering_preserves_ancestors_for_unexpanded_matches() {
    let root = make_temp_dir("grove-path-filter-ancestors");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");

    let mut tab = TabState::new(root.clone());
    let snapshot = build_snapshot(&root).expect("full snapshot should build for the test");
    assert!(
        tab.ingest_path_index_batch(snapshot.entries),
        "preloading the path index should change the tab state for the test"
    );
    tab.complete_path_index();
    tab.set_path_filter_query("nested")
        .expect("setting a path filter query should succeed");

    let rendered_paths = tab
        .tree
        .visible_rows
        .iter()
        .map(|row| {
            tab.tree
                .node(row.node_id)
                .expect("visible node should exist")
                .rel_path
                .clone()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        rendered_paths,
        vec![
            PathBuf::from("."),
            PathBuf::from("alpha"),
            PathBuf::from("alpha/nested.txt"),
        ],
        "filtered tree should keep the root and matching ancestors visible"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn clearing_filter_restores_unfiltered_selection() {
    let root = make_temp_dir("grove-path-filter-restore");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");

    let mut tab = TabState::new(root.clone());
    tab.tree.select_next();
    tab.tree.select_next();
    let selected_before = tab
        .tree
        .visible_rows
        .get(tab.tree.selected_row)
        .and_then(|row| tab.tree.node(row.node_id))
        .expect("selected row should exist before filtering")
        .rel_path
        .clone();
    assert_eq!(selected_before, PathBuf::from("beta.txt"));

    tab.set_path_filter_query("nested")
        .expect("setting a path filter query should succeed");
    tab.set_path_filter_query("")
        .expect("clearing a path filter query should succeed");

    let selected_after = tab
        .tree
        .visible_rows
        .get(tab.tree.selected_row)
        .and_then(|row| tab.tree.node(row.node_id))
        .expect("selected row should exist after clearing the filter")
        .rel_path
        .clone();
    assert_eq!(
        selected_after,
        PathBuf::from("beta.txt"),
        "clearing the filter should restore the prior real-tree selection"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn partial_index_batches_can_produce_matches_before_completion() {
    let root = make_temp_dir("grove-path-filter-partial-batch");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");

    let snapshot = build_snapshot(&root).expect("full snapshot should build for the test");
    let partial_batch = snapshot
        .entries
        .iter()
        .filter(|entry| entry.rel_path.starts_with("alpha"))
        .cloned()
        .collect::<Vec<_>>();

    let mut tab = TabState::new(root.clone());
    tab.set_path_filter_query("nested")
        .expect("setting a path filter query should succeed");
    let initial_paths = tab
        .tree
        .visible_rows
        .iter()
        .map(|row| {
            tab.tree
                .node(row.node_id)
                .expect("visible node should exist")
                .rel_path
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        initial_paths,
        vec![PathBuf::from(".")],
        "before any batches arrive, the filtered tree should only have the root"
    );

    assert!(
        tab.ingest_path_index_batch(partial_batch),
        "ingesting a partial batch should change filter results"
    );
    let updated_paths = tab
        .tree
        .visible_rows
        .iter()
        .map(|row| {
            tab.tree
                .node(row.node_id)
                .expect("visible node should exist")
                .rel_path
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        updated_paths,
        vec![
            PathBuf::from("."),
            PathBuf::from("alpha"),
            PathBuf::from("alpha/nested.txt"),
        ],
        "partial batches should be enough to surface matching rows before indexing completes"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn new_tab_starts_without_background_index_until_demanded() {
    let root = make_temp_dir("grove-path-filter-lazy-start");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");

    let tab = TabState::new(root.clone());

    assert!(
        tab.path_index.receiver.is_none(),
        "lazy-first navigation should not start recursive indexing at tab creation"
    );
    assert!(
        matches!(tab.path_index.status, PathIndexStatus::Idle),
        "tabs should begin with an idle path index until a demand trigger starts it"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn visibility_toggles_rebuild_tree_without_starting_background_index() {
    let root = make_temp_dir("grove-path-filter-visibility-toggle");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");
    fs::write(root.join(".hidden.txt"), "hidden").expect("should create hidden file");

    let mut tab = TabState::new(root.clone());

    assert!(
        tab.toggle_show_hidden()
            .expect("toggling hidden-file visibility should succeed"),
        "show-hidden toggle should rebuild the visible tree"
    );
    assert!(
        tab.path_index.receiver.is_none(),
        "show-hidden toggles should not kick off recursive indexing without a filter/search consumer"
    );
    assert!(
        matches!(tab.path_index.status, PathIndexStatus::Idle),
        "visibility rebuilds without a demand consumer should leave the path index idle"
    );

    assert!(
        tab.toggle_respect_gitignore()
            .expect("toggling gitignore visibility should succeed"),
        "respect-gitignore toggle should rebuild the visible tree"
    );
    assert!(
        tab.path_index.receiver.is_none(),
        "gitignore toggles should not kick off recursive indexing without a filter/search consumer"
    );
    assert!(
        matches!(tab.path_index.status, PathIndexStatus::Idle),
        "visibility rebuilds without a demand consumer should leave the path index idle"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn refresh_after_file_op_preserves_clear_filter_restore_target() {
    let root = make_temp_dir("grove-path-filter-file-op-restore");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta.txt");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("beta.txt"))
    );
    app.tabs[0]
        .set_path_filter_query("alpha")
        .expect("setting a path filter query should succeed");

    app.refresh_active_tab_after_file_op(Some(std::path::Path::new("alpha.txt")))
        .expect("file-op refresh should succeed");
    app.tabs[0]
        .set_path_filter_query("")
        .expect("clearing the path filter query should succeed");

    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(PathBuf::from("alpha.txt")),
        "clearing the filter after a file-op refresh should restore the refreshed unfiltered selection"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_preserves_filter_selection_and_expansion_when_selected_path_survives() {
    let root = make_temp_dir("grove-path-filter-watcher-preserve");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::create_dir_all(root.join("beta")).expect("should create beta dir");
    fs::write(root.join("alpha").join("nested-a.txt"), "alpha")
        .expect("should create nested-a.txt");
    fs::write(root.join("beta").join("nested-b.txt"), "beta").expect("should create nested-b.txt");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("alpha"))
    );
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("alpha/nested-a.txt"))
    );
    app.tabs[0]
        .set_path_filter_query("nested")
        .expect("setting a path filter query should succeed");

    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![PathBuf::from("alpha/nested-a.txt")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should rebuild matching tab state");
    assert_eq!(app.tabs[0].path_filter.query, "nested");
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(PathBuf::from("alpha/nested-a.txt"))
    );
    let visible_paths = visible_rel_paths(&app.tabs[0]);
    assert!(
        visible_paths
            .iter()
            .all(|path| path == &PathBuf::from(".") || path.starts_with("alpha")),
        "watcher refresh should keep visible rows filtered to the matching subtree"
    );
    assert!(
        visible_paths.contains(&PathBuf::from("alpha/nested-a.txt")),
        "filtered rows should still include the selected match"
    );
    assert!(
        app.tabs[0]
            .tree
            .expanded_directory_paths()
            .contains(&PathBuf::from("alpha")),
        "expanded directories should survive a watcher refresh"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_preserves_the_current_filtered_selection_over_the_pre_filter_anchor() {
    let root = make_temp_dir("grove-path-filter-watcher-current-filtered-selection");
    fs::write(root.join("alpha-one.txt"), "alpha one").expect("should create alpha-one");
    fs::write(root.join("alpha-two.txt"), "alpha two").expect("should create alpha-two");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    let snapshot = build_snapshot(&root).expect("full snapshot should build for the test");
    assert!(
        app.tabs[0].ingest_path_index_batch(snapshot.entries),
        "preloading the path index should change the tab state for the test"
    );
    app.tabs[0].complete_path_index();
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("alpha-one.txt"))
    );
    app.tabs[0]
        .set_path_filter_query("alpha")
        .expect("setting a path filter query should succeed");
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("alpha-two.txt")),
        "the user should be able to move to a different filtered match before the refresh"
    );

    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![PathBuf::from("alpha-two.txt")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should rebuild matching tab state");
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(PathBuf::from("alpha-two.txt")),
        "watcher refresh should preserve the current filtered selection instead of snapping back to the pre-filter anchor"
    );
    let visible_paths = visible_rel_paths(&app.tabs[0]);
    assert!(
        visible_paths.contains(&PathBuf::from("alpha-two.txt")),
        "the current filtered match should remain visible"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_clears_active_path_index_receiver_before_reapplying_filter() {
    let root = make_temp_dir("grove-path-filter-watcher-receiver-clear");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested-a.txt"), "alpha")
        .expect("should create nested-a.txt");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("alpha"))
    );
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("alpha/nested-a.txt"))
    );
    app.tabs[0]
        .set_path_filter_query("nested")
        .expect("setting a path filter query should succeed");

    let (sender, receiver) = mpsc::channel();
    app.tabs[0].path_index.receiver = Some(receiver);
    app.tabs[0].path_index.status = PathIndexStatus::Building { indexed_paths: 1 };

    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![PathBuf::from("alpha/nested-a.txt")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should rebuild matching tab state");
    assert!(
        app.tabs[0].path_index.receiver.is_none(),
        "watcher refresh should clear any stale path-index receiver"
    );
    assert!(
        matches!(app.tabs[0].path_index.status, PathIndexStatus::Ready),
        "synthetic watcher refresh should replace background indexing state"
    );
    assert!(
        sender.send(PathIndexEvent::Complete).is_err(),
        "clearing the receiver should prevent stale batches from being delivered"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_falls_back_to_a_visible_selection_when_filtered_recovery_is_hidden() {
    let root = make_temp_dir("grove-path-filter-watcher-filtered-fallback");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta");
    fs::write(root.join("gamma.txt"), "gamma").expect("should create gamma");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("beta.txt"))
    );
    app.tabs[0]
        .set_path_filter_query("alpha")
        .expect("setting a path filter query should succeed");

    fs::remove_file(root.join("beta.txt")).expect("beta should be removed");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            removed_paths: vec![PathBuf::from("beta.txt")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(PathBuf::from("alpha.txt")),
        "watcher refresh should recover to a visible filtered row when the unfiltered fallback is hidden"
    );
    assert!(
        app.status.message.contains("beta.txt"),
        "status should explain which selected path disappeared"
    );
    assert!(
        app.status.message.contains("alpha.txt"),
        "status should name the visible row that was actually selected"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_missing_selected_path_falls_back_to_parent_when_no_sibling_survives() {
    let root = make_temp_dir("grove-path-filter-watcher-parent-fallback");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::write(root.join("docs").join("guide.md"), "guide").expect("should create guide");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("docs"))
    );
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("docs/guide.md"))
    );

    fs::remove_file(root.join("docs").join("guide.md")).expect("guide should be removed");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            removed_paths: vec![PathBuf::from("docs/guide.md")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(PathBuf::from("docs"))
    );
    assert!(
        app.status.message.contains("docs/guide.md"),
        "status should explain which selected path disappeared"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn setting_path_filter_query_starts_background_index_when_snapshot_is_idle() {
    let root = make_temp_dir("grove-path-filter-demand-index");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");

    let mut tab = TabState::new(root.clone());
    tab.path_index = PathIndexState::default();

    tab.set_path_filter_query("alpha")
        .expect("setting a path filter query should succeed");

    assert!(
        tab.path_index.receiver.is_some(),
        "path filtering should start background indexing when no fresh snapshot exists"
    );
    assert!(
        matches!(tab.path_index.status, PathIndexStatus::Building { .. }),
        "starting demand-driven indexing should move the path index into building state"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn poll_path_index_retains_backlog_for_future_ticks() {
    let root = make_temp_dir("grove-path-filter-bounded-poll");
    for index in 0..48 {
        fs::write(root.join(format!("file-{index:03}.txt")), "sample")
            .expect("should create test file");
    }

    let snapshot = build_snapshot(&root).expect("full snapshot should build for the test");
    let total_entries = snapshot.entries.len();
    assert!(
        total_entries > 16,
        "test needs enough entries to exceed one poll budget"
    );

    let (sender, receiver) = mpsc::channel();
    for entry in snapshot.entries {
        sender
            .send(PathIndexEvent::Batch(vec![entry]))
            .expect("batch should queue");
    }
    sender
        .send(PathIndexEvent::Complete)
        .expect("complete event should queue");
    drop(sender);

    let mut tab = TabState::new(root.clone());
    tab.path_index.receiver = Some(receiver);
    tab.path_index.snapshot.entries.clear();
    tab.path_index.status = PathIndexStatus::Building { indexed_paths: 0 };

    assert!(
        tab.poll_path_index().expect("poll should succeed"),
        "processing a bounded subset of batches should still report a change"
    );
    assert!(
        tab.path_index.receiver.is_some(),
        "remaining batches should stay queued for future ticks"
    );
    assert!(
        matches!(tab.path_index.status, PathIndexStatus::Building { .. }),
        "path index should stay in building state until the backlog drains"
    );
    assert!(
        tab.path_index.snapshot.entries.len() < total_entries,
        "a single tick should not drain every queued batch"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn default_index_excludes_hidden_and_gitignored_paths() {
    let root = make_temp_dir("grove-path-filter-visibility");
    fs::create_dir_all(root.join(".git")).expect("should create synthetic git dir");
    fs::write(root.join(".gitignore"), "ignored.txt\n").expect("should create .gitignore");
    fs::write(root.join(".secret"), "secret").expect("should create hidden file");
    fs::write(root.join("ignored.txt"), "ignored").expect("should create ignored file");
    fs::write(root.join("visible.txt"), "visible").expect("should create visible file");

    let snapshot = build_snapshot(&root).expect("default snapshot should build");
    let indexed_paths = snapshot
        .entries
        .iter()
        .map(|entry| entry.rel_path.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        indexed_paths,
        vec![PathBuf::from("visible.txt")],
        "default indexing should hide hidden paths and respect .gitignore"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

fn visible_rel_paths(tab: &TabState) -> Vec<std::path::PathBuf> {
    tab.tree
        .visible_rows
        .iter()
        .filter_map(|row| tab.tree.node(row.node_id).map(|node| node.rel_path.clone()))
        .collect()
}

#[test]
fn unfiltered_batches_refresh_expanded_directory_rows() {
    let root = make_temp_dir("grove-path-filter-unfiltered-batch");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("one.txt"), "one").expect("should create one.txt");
    fs::write(root.join("alpha").join("two.txt"), "two").expect("should create two.txt");

    let snapshot = build_snapshot(&root).expect("full snapshot should build for the test");
    let first_batch = snapshot
        .entries
        .iter()
        .filter(|entry| {
            entry.rel_path == PathBuf::from("alpha")
                || entry.rel_path == PathBuf::from("alpha/one.txt")
        })
        .cloned()
        .collect::<Vec<_>>();
    let second_batch = snapshot
        .entries
        .iter()
        .filter(|entry| entry.rel_path == PathBuf::from("alpha/two.txt"))
        .cloned()
        .collect::<Vec<_>>();

    let mut tab = TabState::new(root.clone());
    assert!(tab.ingest_path_index_batch(first_batch));
    tab.tree.select_next();
    assert!(tab.tree.expand_selected(), "alpha should expand");

    let after_expand = tab
        .tree
        .visible_rows
        .iter()
        .map(|row| {
            tab.tree
                .node(row.node_id)
                .expect("visible node should exist")
                .rel_path
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        after_expand,
        vec![
            PathBuf::from("."),
            PathBuf::from("alpha"),
            PathBuf::from("alpha/one.txt"),
        ],
        "only the first indexed child should be visible before the second batch arrives"
    );

    assert!(tab.ingest_path_index_batch(second_batch));
    let after_second_batch = tab
        .tree
        .visible_rows
        .iter()
        .map(|row| {
            tab.tree
                .node(row.node_id)
                .expect("visible node should exist")
                .rel_path
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        after_second_batch,
        vec![
            PathBuf::from("."),
            PathBuf::from("alpha"),
            PathBuf::from("alpha/one.txt"),
            PathBuf::from("alpha/two.txt"),
        ],
        "later unfiltered batches should refresh visible rows for already-expanded directories"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn filtered_batches_preserve_selected_match() {
    let root = make_temp_dir("grove-path-filter-selection-stability");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::create_dir_all(root.join("beta")).expect("should create beta dir");
    fs::create_dir_all(root.join("gamma")).expect("should create gamma dir");
    fs::write(root.join("alpha").join("nested-a.txt"), "a").expect("should create alpha file");
    fs::write(root.join("beta").join("nested-b.txt"), "b").expect("should create beta file");
    fs::write(root.join("gamma").join("nested-c.txt"), "c").expect("should create gamma file");

    let snapshot = build_snapshot(&root).expect("full snapshot should build for the test");
    let first_batch = snapshot
        .entries
        .iter()
        .filter(|entry| entry.rel_path.starts_with("alpha") || entry.rel_path.starts_with("beta"))
        .cloned()
        .collect::<Vec<_>>();
    let second_batch = snapshot
        .entries
        .iter()
        .filter(|entry| entry.rel_path.starts_with("gamma"))
        .cloned()
        .collect::<Vec<_>>();

    let mut tab = TabState::new(root.clone());
    tab.set_path_filter_query("nested")
        .expect("setting a path filter query should succeed");
    assert!(tab.ingest_path_index_batch(first_batch));
    assert!(
        tab.tree
            .select_rel_path(PathBuf::from("beta/nested-b.txt").as_path()),
        "beta match should be selectable after the first batch"
    );

    assert!(tab.ingest_path_index_batch(second_batch));
    let selected_after = tab
        .tree
        .selected_rel_path()
        .expect("selected path should remain available");
    assert_eq!(
        selected_after,
        PathBuf::from("beta/nested-b.txt"),
        "reapplying a filtered batch should preserve the current selected match when it still exists"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn clearing_filter_after_visibility_toggle_restores_original_unfiltered_selection() {
    let root = make_temp_dir("grove-path-filter-toggle-restore");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");
    fs::write(root.join(".secret"), "secret").expect("should create hidden file");

    let mut tab = TabState::new(root.clone());
    tab.tree.select_next();
    tab.tree.select_next();
    assert_eq!(
        tab.tree.selected_rel_path(),
        Some(PathBuf::from("beta.txt")),
        "beta.txt should be the remembered unfiltered selection"
    );

    tab.set_path_filter_query("nested")
        .expect("setting a path filter query should succeed");
    assert!(
        tab.toggle_show_hidden()
            .expect("toggling hidden visibility should succeed")
    );
    tab.set_path_filter_query("")
        .expect("clearing the path filter query should succeed");

    assert_eq!(
        tab.tree.selected_rel_path(),
        Some(PathBuf::from("beta.txt")),
        "clearing the filter after a visibility toggle should restore the original real-tree selection"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[cfg(unix)]
#[test]
fn raw_visibility_snapshot_does_not_descend_into_symlink_directories() {
    use std::os::unix::fs::symlink;

    let root = make_temp_dir("grove-path-filter-symlink-loop-guard");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");
    symlink(root.join("alpha"), root.join("linked-alpha")).expect("should create dir symlink");

    let snapshot = build_snapshot_with_visibility(
        &root,
        VisibilitySettings {
            show_hidden: false,
            respect_gitignore: false,
        },
    )
    .expect("raw visibility snapshot should build");
    let indexed_paths = snapshot
        .entries
        .iter()
        .map(|entry| entry.rel_path.clone())
        .collect::<Vec<_>>();

    assert!(
        indexed_paths.contains(&PathBuf::from("linked-alpha")),
        "the symlink directory itself should still be indexed"
    );
    assert!(
        !indexed_paths.contains(&PathBuf::from("linked-alpha/nested.txt")),
        "raw visibility indexing should not recurse through symlink directories"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
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
