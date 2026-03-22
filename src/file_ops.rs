use std::ffi::OsString;
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::{Component, Path, PathBuf};

use crate::error::Result;

pub fn relative_path_text(rel_path: &Path) -> String {
    rel_path.display().to_string()
}

pub fn absolute_path_text(root_abs: &Path, rel_path: &Path) -> String {
    root_abs.join(rel_path).display().to_string()
}

pub fn create_file(root_abs: &Path, rel_path: &Path) -> Result<PathBuf> {
    let rel_path = normalize_relative_path(rel_path)?;
    let abs_path = root_abs.join(&rel_path);
    if abs_path.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!("destination already exists: {}", rel_path.display()),
        )
        .into());
    }

    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::File::create(&abs_path)?;
    Ok(rel_path)
}

pub fn create_directory(root_abs: &Path, rel_path: &Path) -> Result<PathBuf> {
    let rel_path = normalize_relative_path(rel_path)?;
    let abs_path = root_abs.join(&rel_path);
    if abs_path.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!("destination already exists: {}", rel_path.display()),
        )
        .into());
    }

    fs::create_dir_all(&abs_path)?;
    Ok(rel_path)
}

pub fn rename_path(root_abs: &Path, source_rel: &Path, dest_rel: &Path) -> Result<PathBuf> {
    move_path(root_abs, source_rel, dest_rel)
}

pub fn duplicate_path(root_abs: &Path, source_rel: &Path, dest_rel: &Path) -> Result<PathBuf> {
    let source_rel = normalize_relative_path(source_rel)?;
    let dest_rel = normalize_relative_path(dest_rel)?;
    ensure_distinct_paths(&source_rel, &dest_rel)?;

    let source_abs = root_abs.join(&source_rel);
    let dest_abs = root_abs.join(&dest_rel);
    if !source_abs.exists() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("source path does not exist: {}", source_rel.display()),
        )
        .into());
    }
    if dest_abs.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!("destination already exists: {}", dest_rel.display()),
        )
        .into());
    }

    copy_entry(&source_abs, &dest_abs)?;
    Ok(dest_rel)
}

pub fn move_path(root_abs: &Path, source_rel: &Path, dest_rel: &Path) -> Result<PathBuf> {
    let source_rel = normalize_relative_path(source_rel)?;
    let dest_rel = normalize_relative_path(dest_rel)?;
    ensure_distinct_paths(&source_rel, &dest_rel)?;

    let source_abs = root_abs.join(&source_rel);
    let dest_abs = root_abs.join(&dest_rel);
    if !source_abs.exists() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("source path does not exist: {}", source_rel.display()),
        )
        .into());
    }
    if dest_abs.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!("destination already exists: {}", dest_rel.display()),
        )
        .into());
    }

    move_path_internal(&source_abs, &dest_abs)?;
    Ok(dest_rel)
}

pub fn trash_path(root_abs: &Path, rel_path: &Path) -> Result<()> {
    let rel_path = normalize_relative_path(rel_path)?;
    let abs_path = root_abs.join(rel_path);
    if !abs_path.exists() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("path does not exist: {}", abs_path.display()),
        )
        .into());
    }

    move_to_trash(&abs_path)?;
    Ok(())
}

fn normalize_relative_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(Error::new(ErrorKind::InvalidInput, "path cannot be empty"));
    }
    if path.is_absolute() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "path must be relative to the active root",
        ));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "path cannot escape the active root",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "path must be relative to the active root",
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "path cannot target the root",
        ));
    }

    Ok(normalized)
}

fn ensure_distinct_paths(source_rel: &Path, dest_rel: &Path) -> std::io::Result<()> {
    if source_rel == dest_rel {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "source and destination must differ",
        ));
    }
    Ok(())
}

fn move_path_internal(source_abs: &Path, dest_abs: &Path) -> std::io::Result<()> {
    if let Some(parent) = dest_abs.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::rename(source_abs, dest_abs) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_entry(source_abs, dest_abs)?;
            verify_copied_entry(source_abs, dest_abs)?;
            move_to_trash(source_abs)?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn move_to_trash(abs_path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        move_to_macos_trash(abs_path)
    }

    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(abs_path).map_err(|err| Error::other(err.to_string()))
    }
}

#[cfg(target_os = "macos")]
fn move_to_macos_trash(abs_path: &Path) -> std::io::Result<()> {
    let file_name = abs_path.file_name().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "path cannot target the filesystem root",
        )
    })?;
    let trash_dir = macos_trash_dir()?;
    fs::create_dir_all(&trash_dir)?;
    let trash_dest = unique_trash_destination(&trash_dir, file_name);
    move_to_trash_destination(abs_path, &trash_dest)
}

#[cfg(target_os = "macos")]
fn macos_trash_dir() -> std::io::Result<PathBuf> {
    if let Some(override_dir) = std::env::var_os("GROVE_TRASH_DIR") {
        return Ok(PathBuf::from(override_dir));
    }

    let home = std::env::var_os("HOME")
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".Trash"))
}

#[cfg(target_os = "macos")]
fn unique_trash_destination(trash_dir: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let candidate = trash_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let mut counter = 2usize;
    loop {
        let mut candidate_name = file_name.to_os_string();
        candidate_name.push(format!(" {counter}"));
        let candidate = trash_dir.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
        counter = counter.saturating_add(1);
    }
}

#[cfg(target_os = "macos")]
fn move_to_trash_destination(source_abs: &Path, trash_dest: &Path) -> std::io::Result<()> {
    if let Some(parent) = trash_dest.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::rename(source_abs, trash_dest) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_entry(source_abs, trash_dest)?;
            verify_copied_entry(source_abs, trash_dest)?;
            remove_entry_permanently(source_abs)?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(target_os = "macos")]
fn remove_entry_permanently(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_entry(source_abs: &Path, dest_abs: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(source_abs)?;
    if metadata.file_type().is_symlink() {
        return copy_symlink(source_abs, dest_abs);
    }
    if metadata.is_dir() {
        fs::create_dir_all(dest_abs)?;
        for child in fs::read_dir(source_abs)? {
            let child = child?;
            let child_dest = dest_abs.join(child.file_name());
            copy_entry(&child.path(), &child_dest)?;
        }
        fs::set_permissions(dest_abs, metadata.permissions())?;
        return Ok(());
    }

    if let Some(parent) = dest_abs.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source_abs, dest_abs)?;
    fs::set_permissions(dest_abs, metadata.permissions())?;
    Ok(())
}

fn verify_copied_entry(source_abs: &Path, dest_abs: &Path) -> std::io::Result<()> {
    let source_meta = fs::symlink_metadata(source_abs)?;
    let dest_meta = fs::symlink_metadata(dest_abs)?;
    if source_meta.file_type().is_symlink() || dest_meta.file_type().is_symlink() {
        let source_target = fs::read_link(source_abs)?;
        let dest_target = fs::read_link(dest_abs)?;
        if source_target != dest_target {
            return Err(Error::other("copied symlink target did not match source"));
        }
        return Ok(());
    }

    if source_meta.is_dir() != dest_meta.is_dir() || source_meta.is_file() != dest_meta.is_file() {
        return Err(Error::other("copied entry type did not match source"));
    }

    if source_meta.is_dir() {
        let mut source_names = read_dir_names(source_abs)?;
        let mut dest_names = read_dir_names(dest_abs)?;
        source_names.sort();
        dest_names.sort();
        if source_names != dest_names {
            return Err(Error::other(
                "copied directory contents did not match source",
            ));
        }
        for child in source_names {
            verify_copied_entry(&source_abs.join(&child), &dest_abs.join(&child))?;
        }
        return Ok(());
    }

    if source_meta.len() != dest_meta.len() || fs::read(source_abs)? != fs::read(dest_abs)? {
        return Err(Error::other("copied file contents did not match source"));
    }
    Ok(())
}

fn read_dir_names(path: &Path) -> std::io::Result<Vec<OsString>> {
    fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect()
}

#[cfg(unix)]
fn copy_symlink(source_abs: &Path, dest_abs: &Path) -> std::io::Result<()> {
    let target = fs::read_link(source_abs)?;
    if let Some(parent) = dest_abs.parent() {
        fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(target, dest_abs)
}

#[cfg(not(unix))]
fn copy_symlink(source_abs: &Path, dest_abs: &Path) -> std::io::Result<()> {
    let _ = (source_abs, dest_abs);
    Err(Error::new(
        ErrorKind::Unsupported,
        "symlink copy is only implemented on unix targets",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn normalize_relative_path_rejects_escape_and_root_targets() {
        assert!(normalize_relative_path(Path::new("../secret")).is_err());
        assert!(normalize_relative_path(Path::new(".")).is_err());
        assert!(normalize_relative_path(Path::new("")).is_err());
        assert_eq!(
            normalize_relative_path(Path::new("./alpha/beta.txt")).expect("path should normalize"),
            PathBuf::from("alpha/beta.txt")
        );
    }

    #[test]
    fn move_path_falls_back_to_copy_verify_and_trash_on_cross_device_errors() {
        let root = make_temp_dir("grove-file-ops-exdev");
        let source = root.join("alpha.txt");
        let dest = root.join("nested").join("beta.txt");
        let trash_dir = root.join(".test-trash");
        fs::write(&source, "alpha").expect("source should exist");

        with_test_trash_dir(&trash_dir, || {
            copy_entry(&source, &dest).expect("copy fallback should succeed");
            verify_copied_entry(&source, &dest).expect("copy verification should succeed");
            move_to_trash(&source).expect("source should move to trash");
        });

        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(&dest).expect("dest should read"),
            "alpha"
        );

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
}
