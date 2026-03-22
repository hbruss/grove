use grove::watcher::{WatchEvent, WatchEventKind, coalesce_refresh_plans, normalize_watched_root};
use std::fs;
use std::path::PathBuf;

#[test]
fn coalesces_events_by_normalized_root_and_buckets_paths() {
    let root = make_temp_dir("grove-watcher-refresh-root");
    let normalized_root = normalize_watched_root(&root);

    let created = root.join("docs/new.txt");
    let changed = root.join("src/lib.rs");
    let removed = root.join(".git/index");
    fs::create_dir_all(created.parent().expect("created path should have parent"))
        .expect("should create docs dir");
    fs::create_dir_all(changed.parent().expect("changed path should have parent"))
        .expect("should create src dir");
    fs::create_dir_all(removed.parent().expect("removed path should have parent"))
        .expect("should create .git dir");

    let events = vec![
        WatchEvent::new(&root, WatchEventKind::Create, &created),
        WatchEvent::new(&root.join("."), WatchEventKind::Change, &changed),
        WatchEvent::new(&root, WatchEventKind::Remove, &removed),
    ];

    let plans = coalesce_refresh_plans(events);
    assert_eq!(plans.len(), 1);

    let plan = &plans[0];
    assert_eq!(plan.root, normalized_root);
    assert_eq!(plan.created_paths, vec![PathBuf::from("docs/new.txt")]);
    assert_eq!(plan.changed_paths, vec![PathBuf::from("src/lib.rs")]);
    assert_eq!(plan.removed_paths, vec![PathBuf::from(".git/index")]);
    assert!(plan.git_dirty);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn ignores_events_outside_the_watched_root() {
    let root = make_temp_dir("grove-watcher-refresh-ignored");
    let other = make_temp_dir("grove-watcher-refresh-other");
    let changed = other.join("outside.txt");
    fs::write(&changed, "outside").expect("should create outside file");

    let plans = coalesce_refresh_plans(vec![WatchEvent::new(
        &root,
        WatchEventKind::Change,
        &changed,
    )]);

    assert!(plans.is_empty());

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(other).expect("temp root should be removed");
}

#[test]
fn marks_git_dirty_for_repo_content_and_git_metadata_changes() {
    let root = make_temp_dir("grove-watcher-refresh-git-dirty");
    let config = root.join("config.toml");
    let git_head = root.join(".git/HEAD");
    fs::create_dir_all(
        git_head
            .parent()
            .expect("git metadata should have a parent"),
    )
    .expect("should create git metadata dir");
    fs::write(&config, "theme = \"dark\"").expect("should create config");
    fs::write(&git_head, "ref: refs/heads/main").expect("should create git head");

    let plans = coalesce_refresh_plans(vec![
        WatchEvent::new(&root, WatchEventKind::Change, &config),
        WatchEvent::new(&root, WatchEventKind::Change, &git_head),
    ]);

    assert_eq!(plans.len(), 1);
    assert!(plans[0].git_dirty);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn collapses_contradictory_sequences_into_the_last_event_kind() {
    let root = make_temp_dir("grove-watcher-refresh-contradictory");
    let path = root.join("docs/notes.txt");
    fs::create_dir_all(path.parent().expect("path should have parent"))
        .expect("should create parent dir");

    let plans = coalesce_refresh_plans(vec![
        WatchEvent::new(&root, WatchEventKind::Create, &path),
        WatchEvent::new(&root, WatchEventKind::Change, &path),
        WatchEvent::new(&root, WatchEventKind::Remove, &path),
    ]);

    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert_eq!(plan.created_paths, Vec::<PathBuf>::new());
    assert_eq!(plan.changed_paths, Vec::<PathBuf>::new());
    assert_eq!(plan.removed_paths, vec![PathBuf::from("docs/notes.txt")]);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn later_events_replace_earlier_kinds_for_the_same_path() {
    let root = make_temp_dir("grove-watcher-refresh-replacement");
    let path = root.join("docs/notes.txt");
    fs::create_dir_all(path.parent().expect("path should have parent"))
        .expect("should create parent dir");

    let plans = coalesce_refresh_plans(vec![
        WatchEvent::new(&root, WatchEventKind::Remove, &path),
        WatchEvent::new(&root, WatchEventKind::Create, &path),
        WatchEvent::new(&root, WatchEventKind::Change, &path),
    ]);

    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert_eq!(plan.created_paths, Vec::<PathBuf>::new());
    assert_eq!(plan.changed_paths, vec![PathBuf::from("docs/notes.txt")]);
    assert_eq!(plan.removed_paths, Vec::<PathBuf>::new());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

fn make_temp_dir(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&root).expect("temp dir should be created");
    root
}

fn unique_suffix() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough for tests")
        .as_nanos()
}
