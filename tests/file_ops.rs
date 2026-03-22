use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use grove::error::GroveError;
use grove::file_ops;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn helpers_render_relative_and_absolute_paths() {
    let root = Path::new("/tmp/demo");
    let rel = Path::new("nested/file.txt");

    assert_eq!(file_ops::relative_path_text(rel), "nested/file.txt");
    assert_eq!(
        file_ops::absolute_path_text(root, rel),
        "/tmp/demo/nested/file.txt"
    );
}

#[test]
fn create_file_directory_rename_duplicate_move_and_trash_work_end_to_end() {
    let root = make_temp_dir("grove-file-ops");
    let trash_dir = root.join(".test-trash");

    with_test_trash_dir(&trash_dir, || {
        let created_file = file_ops::create_file(&root, Path::new("notes/alpha.txt"))
            .expect("file should be created");
        assert_eq!(created_file, PathBuf::from("notes/alpha.txt"));
        assert_eq!(
            fs::read_to_string(root.join("notes/alpha.txt")).unwrap(),
            ""
        );

        fs::write(root.join("notes/alpha.txt"), "alpha").expect("file should be writable");

        let created_dir = file_ops::create_directory(&root, Path::new("docs/reference"))
            .expect("directory should be created");
        assert_eq!(created_dir, PathBuf::from("docs/reference"));
        assert!(root.join("docs/reference").is_dir());

        let renamed = file_ops::rename_path(
            &root,
            Path::new("notes/alpha.txt"),
            Path::new("notes/beta.txt"),
        )
        .expect("file should rename");
        assert_eq!(renamed, PathBuf::from("notes/beta.txt"));
        assert!(!root.join("notes/alpha.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("notes/beta.txt")).unwrap(),
            "alpha"
        );

        let duplicated = file_ops::duplicate_path(
            &root,
            Path::new("notes/beta.txt"),
            Path::new("notes/gamma.txt"),
        )
        .expect("file should duplicate");
        assert_eq!(duplicated, PathBuf::from("notes/gamma.txt"));
        assert_eq!(
            fs::read_to_string(root.join("notes/gamma.txt")).unwrap(),
            "alpha"
        );

        let moved = file_ops::move_path(
            &root,
            Path::new("notes/gamma.txt"),
            Path::new("docs/reference/gamma.txt"),
        )
        .expect("file should move");
        assert_eq!(moved, PathBuf::from("docs/reference/gamma.txt"));
        assert!(!root.join("notes/gamma.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("docs/reference/gamma.txt")).unwrap(),
            "alpha"
        );

        file_ops::trash_path(&root, Path::new("docs/reference/gamma.txt"))
            .expect("file should move to trash");
        assert!(!root.join("docs/reference/gamma.txt").exists());
        assert!(trash_dir.exists());
    });

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn duplicate_directory_copies_nested_contents_recursively() {
    let root = make_temp_dir("grove-file-ops-dir-copy");
    fs::create_dir_all(root.join("source/subdir")).expect("source tree should exist");
    fs::write(root.join("source/subdir/leaf.txt"), "leaf").expect("leaf file should exist");

    let duplicated =
        file_ops::duplicate_path(&root, Path::new("source"), Path::new("mirror/source-copy"))
            .expect("directory should duplicate recursively");
    assert_eq!(duplicated, PathBuf::from("mirror/source-copy"));
    assert_eq!(
        fs::read_to_string(root.join("mirror/source-copy/subdir/leaf.txt")).unwrap(),
        "leaf"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn create_duplicate_move_and_trash_reject_invalid_targets() {
    let root = make_temp_dir("grove-file-ops-invalid");
    fs::write(root.join("alpha.txt"), "alpha").expect("source should exist");

    let create_err = file_ops::create_file(&root, Path::new("alpha.txt"))
        .expect_err("existing file should reject create");
    assert!(matches!(
        create_err,
        GroveError::Io(ref err) if err.kind() == std::io::ErrorKind::AlreadyExists
    ));

    let duplicate_err =
        file_ops::duplicate_path(&root, Path::new("alpha.txt"), Path::new("alpha.txt"))
            .expect_err("duplicate should reject same source and destination");
    assert!(matches!(
        duplicate_err,
        GroveError::Io(ref err) if err.kind() == std::io::ErrorKind::InvalidInput
    ));

    let move_err = file_ops::move_path(&root, Path::new("alpha.txt"), Path::new("alpha.txt"))
        .expect_err("move should reject same source and destination");
    assert!(matches!(
        move_err,
        GroveError::Io(ref err) if err.kind() == std::io::ErrorKind::InvalidInput
    ));

    let trash_err = file_ops::trash_path(&root, Path::new("missing.txt"))
        .expect_err("trash should reject missing paths");
    assert!(matches!(
        trash_err,
        GroveError::Io(ref err) if err.kind() == std::io::ErrorKind::NotFound
    ));

    let absolute_err = file_ops::create_directory(&root, Path::new("/tmp/illegal"))
        .expect_err("absolute targets should be rejected");
    assert!(matches!(
        absolute_err,
        GroveError::Io(ref err) if err.kind() == std::io::ErrorKind::InvalidInput
    ));

    fs::remove_dir_all(root).expect("temp root should be removed");
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

fn with_test_trash_dir<T>(trash_dir: &Path, f: impl FnOnce() -> T) -> T {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("trash-dir lock should not be poisoned");
    let previous = std::env::var_os("GROVE_TRASH_DIR");
    unsafe { std::env::set_var("GROVE_TRASH_DIR", trash_dir) };
    let restore = EnvRestore {
        key: "GROVE_TRASH_DIR",
        previous,
    };
    let result = f();
    drop(restore);
    result
}

struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            unsafe { std::env::set_var(self.key, previous) };
        } else {
            unsafe { std::env::remove_var(self.key) };
        }
    }
}
