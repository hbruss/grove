use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{IndexAddOption, Repository, Signature};
use grove::git::backend::{
    GitBackend, GitChange, GitPathStatus, LibgitBackend, RepoHandle, summarize_repo_statuses,
};
#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn discovers_repo_from_nested_directory() {
    let root = make_temp_dir("grove-git-backend-discover");
    let nested = root.join("src").join("nested");
    fs::create_dir_all(&nested).expect("nested path should exist");
    let _repo = Repository::init(&root).expect("repo should initialize");

    let backend = LibgitBackend;
    let repo = backend
        .discover_repo(&nested)
        .expect("repo discovery should succeed")
        .expect("repo should be found");

    assert_eq!(
        repo.root
            .canonicalize()
            .expect("repo root should canonicalize"),
        root.canonicalize().expect("root should canonicalize")
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn discovers_symbolic_branch_name_for_unborn_repo() {
    let root = make_temp_dir("grove-git-backend-unborn-branch");
    let repo = Repository::init(&root).expect("repo should initialize");
    let expected_branch = repo
        .find_reference("HEAD")
        .expect("head ref should exist")
        .symbolic_target()
        .and_then(|target| target.rsplit('/').next())
        .expect("head should point at a symbolic branch")
        .to_string();

    let backend = LibgitBackend;
    let handle = backend
        .discover_repo(&root)
        .expect("repo discovery should succeed")
        .expect("repo should be found");

    assert_eq!(handle.branch_name, expected_branch);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn status_map_reports_repo_relative_index_and_worktree_state() {
    let root = make_temp_dir("grove-git-backend-status");
    let repo = Repository::init(&root).expect("repo should initialize");

    write_file(&root, "tracked.txt", "tracked\n");
    write_file(&root, "both.txt", "base\n");
    commit_all(&repo, "initial");

    write_file(&root, "tracked.txt", "tracked change\n");
    write_file(&root, "staged.txt", "staged addition\n");
    write_file(&root, "both.txt", "staged version\n");
    stage_paths(&repo, &[Path::new("staged.txt"), Path::new("both.txt")]);
    write_file(&root, "both.txt", "worktree version\n");
    write_file(&root, "untracked.txt", "untracked\n");

    let backend = LibgitBackend;
    let handle = backend
        .discover_repo(&root)
        .expect("repo discovery should succeed")
        .expect("repo should be found");
    let status_map = backend.status_map(&handle).expect("status map should load");
    assert!(
        status_map.keys().all(|path| path.is_relative()),
        "status map keys should stay repo-relative"
    );

    let tracked = status_map
        .get(Path::new("tracked.txt"))
        .expect("tracked file should be present");
    assert_eq!(tracked.index, GitChange::Unmodified);
    assert_eq!(tracked.worktree, GitChange::Modified);

    let staged = status_map
        .get(Path::new("staged.txt"))
        .expect("staged file should be present");
    assert_eq!(staged.index, GitChange::Added);
    assert_eq!(staged.worktree, GitChange::Unmodified);

    let both = status_map
        .get(Path::new("both.txt"))
        .expect("mixed-state file should be present");
    assert_eq!(both.index, GitChange::Modified);
    assert_eq!(both.worktree, GitChange::Modified);

    let untracked = status_map
        .get(Path::new("untracked.txt"))
        .expect("untracked file should be present");
    assert!(untracked.untracked);
    assert_eq!(untracked.index, GitChange::Unmodified);
    assert_eq!(untracked.worktree, GitChange::Unmodified);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn diff_for_path_returns_unified_diff_for_modified_file() {
    let root = make_temp_dir("grove-git-backend-diff");
    let repo = Repository::init(&root).expect("repo should initialize");

    write_file(&root, "tracked.txt", "before\n");
    commit_all(&repo, "initial");
    write_file(&root, "tracked.txt", "after\n");

    let backend = LibgitBackend;
    let handle = backend
        .discover_repo(&root)
        .expect("repo discovery should succeed")
        .expect("repo should be found");
    let diff = backend
        .diff_for_path(
            &handle,
            Path::new("tracked.txt"),
            grove::git::backend::DiffMode::Unstaged,
        )
        .expect("diff should load");

    assert!(diff.text.contains("tracked.txt"));
    assert!(diff.text.contains("-before"));
    assert!(diff.text.contains("+after"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn stage_path_updates_modified_untracked_and_deleted_files_in_the_index() {
    let root = make_temp_dir("grove-git-backend-stage");
    let repo = Repository::init(&root).expect("repo should initialize");

    write_file(&root, "tracked.txt", "before\n");
    write_file(&root, "deleted.txt", "gone soon\n");
    commit_all(&repo, "initial");

    write_file(&root, "tracked.txt", "after\n");
    write_file(&root, "untracked.txt", "fresh\n");
    fs::remove_file(root.join("deleted.txt")).expect("tracked file should be removed");

    let backend = LibgitBackend;
    let handle = backend
        .discover_repo(&root)
        .expect("repo discovery should succeed")
        .expect("repo should be found");

    backend
        .stage_path(&handle, Path::new("tracked.txt"))
        .expect("modified file should stage");
    backend
        .stage_path(&handle, Path::new("untracked.txt"))
        .expect("untracked file should stage");
    backend
        .stage_path(&handle, Path::new("deleted.txt"))
        .expect("deleted file should stage");

    let status_map = backend.status_map(&handle).expect("status map should load");
    let tracked = status_map
        .get(Path::new("tracked.txt"))
        .expect("tracked file should be present");
    assert_eq!(tracked.index, GitChange::Modified);
    assert_eq!(tracked.worktree, GitChange::Unmodified);

    let untracked = status_map
        .get(Path::new("untracked.txt"))
        .expect("untracked file should be present");
    assert_eq!(untracked.index, GitChange::Added);
    assert_eq!(untracked.worktree, GitChange::Unmodified);

    let deleted = status_map
        .get(Path::new("deleted.txt"))
        .expect("deleted file should be present");
    assert_eq!(deleted.index, GitChange::Deleted);
    assert_eq!(deleted.worktree, GitChange::Unmodified);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn unstage_path_restores_worktree_only_changes_for_staged_file() {
    let root = make_temp_dir("grove-git-backend-unstage");
    let repo = Repository::init(&root).expect("repo should initialize");

    write_file(&root, "tracked.txt", "before\n");
    commit_all(&repo, "initial");
    write_file(&root, "tracked.txt", "after\n");

    let backend = LibgitBackend;
    let handle = backend
        .discover_repo(&root)
        .expect("repo discovery should succeed")
        .expect("repo should be found");

    backend
        .stage_path(&handle, Path::new("tracked.txt"))
        .expect("modified file should stage");
    backend
        .unstage_path(&handle, Path::new("tracked.txt"))
        .expect("staged file should unstage");

    let status_map = backend.status_map(&handle).expect("status map should load");
    let tracked = status_map
        .get(Path::new("tracked.txt"))
        .expect("tracked file should be present");
    assert_eq!(tracked.index, GitChange::Unmodified);
    assert_eq!(tracked.worktree, GitChange::Modified);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn summarize_repo_statuses_counts_cached_git_categories() {
    let repo = RepoHandle {
        root: PathBuf::from("/repo"),
        branch_name: "main".to_string(),
    };
    let status_map = HashMap::from([
        (
            PathBuf::from("staged.txt"),
            GitPathStatus {
                index: GitChange::Added,
                ..GitPathStatus::default()
            },
        ),
        (
            PathBuf::from("unstaged.txt"),
            GitPathStatus {
                worktree: GitChange::Modified,
                ..GitPathStatus::default()
            },
        ),
        (
            PathBuf::from("both.txt"),
            GitPathStatus {
                index: GitChange::Modified,
                worktree: GitChange::Modified,
                ..GitPathStatus::default()
            },
        ),
        (
            PathBuf::from("untracked.txt"),
            GitPathStatus {
                untracked: true,
                ..GitPathStatus::default()
            },
        ),
        (
            PathBuf::from("conflicted.txt"),
            GitPathStatus {
                conflicted: true,
                ..GitPathStatus::default()
            },
        ),
    ]);

    let summary = summarize_repo_statuses(&repo, &status_map);
    assert_eq!(summary.repo_root, PathBuf::from("/repo"));
    assert_eq!(summary.branch_name, "main");
    assert_eq!(summary.staged_paths, 2);
    assert_eq!(summary.unstaged_paths, 2);
    assert_eq!(summary.untracked_paths, 1);
    assert_eq!(summary.conflicted_paths, 1);
}

#[cfg(unix)]
#[test]
fn stage_path_treats_broken_symlink_as_a_file_entry() {
    let root = make_temp_dir("grove-git-backend-stage-broken-symlink");
    Repository::init(&root).expect("repo should initialize");
    symlink("missing-target.txt", root.join("broken-link.txt"))
        .expect("broken symlink should be created");

    let backend = LibgitBackend;
    let handle = backend
        .discover_repo(&root)
        .expect("repo discovery should succeed")
        .expect("repo should be found");

    backend
        .stage_path(&handle, Path::new("broken-link.txt"))
        .expect("broken symlink should stage");

    let status_map = backend.status_map(&handle).expect("status map should load");
    let symlink_status = status_map
        .get(Path::new("broken-link.txt"))
        .expect("broken symlink should be present");
    assert_eq!(symlink_status.index, GitChange::Added);
    assert_eq!(symlink_status.worktree, GitChange::Unmodified);

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
