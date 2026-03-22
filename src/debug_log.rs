use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DEBUG_LOG_ENV: &str = "GROVE_DEBUG_LOG";

fn write_lock() -> &'static Mutex<()> {
    static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

pub fn env_var_name() -> &'static str {
    DEBUG_LOG_ENV
}

pub fn log(message: &str) {
    let Some(path) = std::env::var_os(DEBUG_LOG_ENV) else {
        return;
    };
    let Ok(_guard) = write_lock().lock() else {
        return;
    };
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let line = format!(
        "ts_ms={timestamp_ms} thread={:?} {message}\n",
        std::thread::current().id()
    );
    let _ = append_line(Path::new(&path), &line);
}

pub fn log_component(component: &str, message: &str) {
    log(&format!("component={component} {message}"));
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    #[test]
    fn append_line_creates_parent_dirs_and_appends() {
        let root = make_temp_dir("grove-debug-log");
        let log_path = root.join("nested").join("grove.log");

        append_line(&log_path, "first\n").expect("first line should be written");
        append_line(&log_path, "second\n").expect("second line should be written");

        let written = fs::read_to_string(&log_path).expect("log file should be readable");
        assert_eq!(written, "first\nsecond\n");

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    fn make_temp_dir(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{label}-{unique}"));
        fs::create_dir_all(&root).expect("temp root should be created");
        root
    }
}
