use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{IndexAddOption, Repository, Signature, build::CheckoutBuilder};
use grove::actions::catalog::{action_bar_entries, command_palette_entries};
use grove::app::{App, PathIndexStatus, TabState};
use grove::state::ContextMode;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn default_action_bar_hides_git_mutation_actions() {
    let app = App::default();

    let action_bar = labels(action_bar_entries(&app));

    assert!(!contains_label(&action_bar, "diff"));
    assert!(!contains_label(&action_bar, "stage"));
    assert!(!contains_label(&action_bar, "unstage"));
}

#[test]
fn catalog_hides_git_mutation_actions_outside_repo_and_for_clean_files() {
    let outside_root = make_temp_dir("grove-action-catalog-outside-repo");
    write_file(&outside_root, "note.txt", "note\n");
    let outside_repo_app = app_with_selected_file(&outside_root, "note.txt");
    assert_git_mutation_labels(&outside_repo_app, false, false);
    assert_diff_label(&outside_repo_app, false);

    let clean_root = make_temp_dir("grove-action-catalog-clean");
    let clean_repo = Repository::init(&clean_root).expect("repo should initialize");
    write_file(&clean_root, "tracked.txt", "tracked\n");
    commit_all(&clean_repo, "initial");
    let mut clean_app = app_with_selected_file(&clean_root, "tracked.txt");
    clean_app
        .refresh_active_git_state()
        .expect("clean repo git state should load");
    assert_git_mutation_labels(&clean_app, false, false);
    assert_diff_label(&clean_app, false);

    fs::remove_dir_all(outside_root).expect("temp root should be removed");
    fs::remove_dir_all(clean_root).expect("temp root should be removed");
}

#[test]
fn catalog_matches_git_mutation_availability_matrix() {
    let modified_root = make_temp_dir("grove-action-catalog-modified");
    let modified_repo = Repository::init(&modified_root).expect("repo should initialize");
    write_file(&modified_root, "tracked.txt", "before\n");
    commit_all(&modified_repo, "initial");
    write_file(&modified_root, "tracked.txt", "after\n");
    let mut modified_app = app_with_selected_file(&modified_root, "tracked.txt");
    modified_app
        .refresh_active_git_state()
        .expect("modified repo git state should load");
    assert_git_mutation_labels(&modified_app, true, false);
    assert_diff_label(&modified_app, true);

    let staged_root = make_temp_dir("grove-action-catalog-staged");
    let staged_repo = Repository::init(&staged_root).expect("repo should initialize");
    write_file(&staged_root, "tracked.txt", "before\n");
    commit_all(&staged_repo, "initial");
    write_file(&staged_root, "tracked.txt", "after\n");
    stage_paths(&staged_repo, &[Path::new("tracked.txt")]);
    let mut staged_app = app_with_selected_file(&staged_root, "tracked.txt");
    staged_app
        .refresh_active_git_state()
        .expect("staged repo git state should load");
    assert_git_mutation_labels(&staged_app, false, true);
    assert_diff_label(&staged_app, false);

    let mixed_root = make_temp_dir("grove-action-catalog-mixed");
    let mixed_repo = Repository::init(&mixed_root).expect("repo should initialize");
    write_file(&mixed_root, "tracked.txt", "before\n");
    commit_all(&mixed_repo, "initial");
    write_file(&mixed_root, "tracked.txt", "staged\n");
    stage_paths(&mixed_repo, &[Path::new("tracked.txt")]);
    write_file(&mixed_root, "tracked.txt", "worktree\n");
    let mut mixed_app = app_with_selected_file(&mixed_root, "tracked.txt");
    mixed_app
        .refresh_active_git_state()
        .expect("mixed repo git state should load");
    assert_git_mutation_labels(&mixed_app, true, true);
    assert_diff_label(&mixed_app, true);

    let conflicted_root = make_temp_dir("grove-action-catalog-conflicted");
    let conflicted_repo = Repository::init(&conflicted_root).expect("repo should initialize");
    write_file(&conflicted_root, "tracked.txt", "base\n");
    commit_all(&conflicted_repo, "initial");
    create_merge_conflict(&conflicted_repo, &conflicted_root, "tracked.txt");
    let mut conflicted_app = app_with_selected_file(&conflicted_root, "tracked.txt");
    conflicted_app
        .refresh_active_git_state()
        .expect("conflicted repo git state should load");
    assert_git_mutation_labels(&conflicted_app, false, false);
    assert_diff_label(&conflicted_app, false);

    fs::remove_dir_all(modified_root).expect("temp root should be removed");
    fs::remove_dir_all(staged_root).expect("temp root should be removed");
    fs::remove_dir_all(mixed_root).expect("temp root should be removed");
    fs::remove_dir_all(conflicted_root).expect("temp root should be removed");
}

#[test]
fn catalog_surfaces_bookmark_and_tab_actions_when_meaningful() {
    let root = make_temp_dir("grove-action-catalog-bookmarks");
    write_file(&root, "tracked.txt", "tracked\n");

    let mut app = app_with_selected_file(&root, "tracked.txt");
    assert!(contains_label(
        &labels(command_palette_entries(&app, "")),
        "pin active root"
    ));
    assert!(!contains_label(
        &labels(command_palette_entries(&app, "")),
        "unpin active root"
    ));
    assert!(!contains_label(
        &labels(command_palette_entries(&app, "")),
        "close tab"
    ));

    assert!(app.pin_active_root());
    assert!(!contains_label(
        &labels(command_palette_entries(&app, "")),
        "pin active root"
    ));
    assert!(contains_label(
        &labels(command_palette_entries(&app, "")),
        "unpin active root"
    ));

    let second_root = make_temp_dir("grove-action-catalog-second-tab");
    write_file(&second_root, "note.txt", "note\n");
    assert!(app.open_or_activate_tab(second_root.clone()));
    assert!(contains_label(
        &labels(command_palette_entries(&app, "")),
        "close tab"
    ));

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(second_root).expect("temp root should be removed");
}

#[test]
fn catalog_surfaces_phase_eight_file_ops_when_selection_allows_them() {
    let root = make_temp_dir("grove-action-catalog-file-ops");
    write_file(&root, "alpha.txt", "alpha\n");

    let file_app = app_with_selected_file(&root, "alpha.txt");
    let palette = labels(command_palette_entries(&file_app, ""));

    for label in [
        "new file",
        "new directory",
        "rename",
        "duplicate",
        "move",
        "trash",
        "reveal in finder",
        "copy relative path",
        "copy absolute path",
    ] {
        assert!(
            contains_label(&palette, label),
            "expected palette to contain {label}: {palette:?}"
        );
    }

    let root_app = {
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        app.tabs[0].tree.selected_row = 0;
        app
    };
    let root_palette = labels(command_palette_entries(&root_app, ""));
    assert!(contains_label(&root_palette, "new file"));
    assert!(contains_label(&root_palette, "new directory"));
    assert!(
        index_of_label(&palette, "open in editor") < index_of_label(&palette, "AI target"),
        "selection actions should rank ahead of global target actions: {palette:?}"
    );
    assert!(
        index_of_label(&palette, "rename") < index_of_label(&palette, "pin active root"),
        "selection actions should rank ahead of root actions: {palette:?}"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn catalog_targets_selected_directory_root_for_bookmark_actions() {
    let root = make_temp_dir("grove-action-catalog-selected-root");
    fs::create_dir_all(root.join("docs")).expect("docs directory should be created");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = PathIndexStatus::Ready;
    assert!(app.tabs[0].tree.select_rel_path(Path::new("docs")));

    let palette = labels(command_palette_entries(&app, ""));
    assert!(contains_label(&palette, "pin selected root"));
    assert!(!contains_label(&palette, "pin active root"));
    assert!(!contains_label(&palette, "unpin active root"));

    app.config
        .bookmarks
        .pins
        .push(fs::canonicalize(&root).expect("root should canonicalize"));
    let pinned_root_palette = labels(command_palette_entries(&app, ""));
    assert!(contains_label(&pinned_root_palette, "pin selected root"));
    assert!(!contains_label(&pinned_root_palette, "unpin active root"));

    app.config.bookmarks.pins.push(
        fs::canonicalize(root.join("docs")).expect("selected directory root should canonicalize"),
    );
    let selected_pinned_palette = labels(command_palette_entries(&app, ""));
    assert!(
        contains_label(&selected_pinned_palette, "unpin selected root"),
        "expected selected directory bookmark action in {selected_pinned_palette:?}"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn unavailable_diff_keeps_current_mode_stable_and_success_clears_stale_status() {
    let root = make_temp_dir("grove-action-catalog-diff-guard");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_file(&root, "clean.txt", "clean\n");
    write_file(&root, "dirty.txt", "before\n");
    commit_all(&repo, "initial");
    write_file(&root, "dirty.txt", "after\n");

    let mut app = app_with_selected_file(&root, "clean.txt");
    app.refresh_active_git_state()
        .expect("git state should load for diff guard");
    app.tabs[0].mode = ContextMode::SearchResults;

    assert!(app.activate_diff_mode_if_available());
    assert_eq!(app.tabs[0].mode, ContextMode::SearchResults);
    assert_eq!(
        app.status.message,
        "diff unavailable: select a modified or untracked file"
    );

    assert!(app.tabs[0].tree.select_rel_path(Path::new("dirty.txt")));
    assert!(
        app.activate_diff_mode_if_available(),
        "successful diff activation should clear the stale denial status"
    );
    assert_eq!(app.tabs[0].mode, ContextMode::Diff);
    assert!(app.status.message.is_empty());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

fn assert_git_mutation_labels(app: &App, expect_stage: bool, expect_unstage: bool) {
    let action_bar = labels(action_bar_entries(app));
    let palette = labels(command_palette_entries(app, ""));

    assert_eq!(
        contains_label(&action_bar, "stage"),
        expect_stage,
        "unexpected action-bar stage availability: {action_bar:?}"
    );
    assert_eq!(
        contains_label(&action_bar, "unstage"),
        expect_unstage,
        "unexpected action-bar unstage availability: {action_bar:?}"
    );
    assert_eq!(
        contains_label(&palette, "stage selected path"),
        expect_stage,
        "unexpected palette stage availability: {palette:?}"
    );
    assert_eq!(
        contains_label(&palette, "unstage selected path"),
        expect_unstage,
        "unexpected palette unstage availability: {palette:?}"
    );
}

fn assert_diff_label(app: &App, expect_diff: bool) {
    let action_bar = labels(action_bar_entries(app));
    let palette = labels(command_palette_entries(app, ""));

    assert_eq!(
        contains_label(&action_bar, "diff"),
        expect_diff,
        "unexpected action-bar diff availability: {action_bar:?}"
    );
    assert_eq!(
        contains_label(&palette, "diff"),
        expect_diff,
        "unexpected palette diff availability: {palette:?}"
    );
}

fn labels(entries: Vec<grove::action::ActionDescriptor>) -> Vec<String> {
    entries.into_iter().map(|entry| entry.label).collect()
}

fn contains_label(labels: &[String], target: &str) -> bool {
    labels.iter().any(|label| label == target)
}

fn index_of_label(labels: &[String], target: &str) -> usize {
    labels
        .iter()
        .position(|label| label == target)
        .unwrap_or_else(|| panic!("expected label {target} in {labels:?}"))
}

fn app_with_selected_file(root: &Path, rel_path: &str) -> App {
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.to_path_buf());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = PathIndexStatus::Ready;
    assert!(
        app.tabs[0].tree.select_rel_path(Path::new(rel_path)),
        "selected path should exist in the tree"
    );
    app
}

fn make_temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("{label}-{unique}-{counter}"));
    fs::create_dir_all(&root).expect("temp root should be created");
    root
}

fn write_file(root: &Path, rel_path: &str, contents: &str) {
    let path = root.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, contents).expect("file should be written");
}

fn stage_paths(repo: &Repository, rel_paths: &[&Path]) {
    let mut index = repo.index().expect("index should open");
    index
        .add_all(rel_paths.iter().copied(), IndexAddOption::DEFAULT, None)
        .expect("paths should stage");
    index.write().expect("index should flush");
}

fn commit_all(repo: &Repository, message: &str) {
    let mut index = repo.index().expect("index should open");
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .expect("repo contents should stage");
    index.write().expect("index should flush");

    let tree_oid = index.write_tree().expect("tree should write");
    let tree = repo.find_tree(tree_oid).expect("tree should load");
    let signature =
        Signature::now("Grove Tests", "grove-tests@example.com").expect("signature should build");

    let parent = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .map(|oid| repo.find_commit(oid).expect("parent commit should load"));

    match parent.as_ref() {
        Some(parent_commit) => {
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[parent_commit],
            )
            .expect("commit should succeed");
        }
        None => {
            repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
                .expect("initial commit should succeed");
        }
    }
}

fn create_merge_conflict(repo: &Repository, root: &Path, rel_path: &str) {
    let base_ref = repo
        .head()
        .expect("head should exist")
        .name()
        .expect("head name should exist")
        .to_string();
    let base_commit = repo
        .head()
        .expect("head should exist")
        .peel_to_commit()
        .expect("head commit should load");
    repo.branch("feature-conflict", &base_commit, false)
        .expect("feature branch should be created");

    write_file(root, rel_path, "main change\n");
    commit_all(repo, "main change");

    repo.set_head("refs/heads/feature-conflict")
        .expect("head should switch to feature");
    repo.checkout_head(Some(CheckoutBuilder::new().force()))
        .expect("feature checkout should succeed");
    write_file(root, rel_path, "feature change\n");
    commit_all(repo, "feature change");

    repo.set_head(&base_ref)
        .expect("head should switch back to base branch");
    repo.checkout_head(Some(CheckoutBuilder::new().force()))
        .expect("base checkout should succeed");

    let feature_commit = repo
        .find_reference("refs/heads/feature-conflict")
        .expect("feature ref should exist")
        .peel_to_commit()
        .expect("feature commit should load");
    let annotated = repo
        .find_annotated_commit(feature_commit.id())
        .expect("annotated commit should load");
    repo.merge(&[&annotated], None, None)
        .expect("merge should produce conflicts");
}
