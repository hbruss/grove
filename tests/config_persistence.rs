use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use grove::config::Config;

#[test]
fn config_load_from_missing_path_returns_defaults() {
    let root = make_temp_dir("grove-config-missing");
    let path = root.join("config.toml");

    let config =
        Config::load_from_path(&path).expect("missing config should fall back to defaults");

    assert!(!config.general.show_hidden);
    assert!(config.general.respect_gitignore);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn config_save_and_reload_roundtrips_visibility_preferences() {
    let root = make_temp_dir("grove-config-roundtrip");
    let path = root.join("config.toml");
    let mut config = Config::default();
    config.general.show_hidden = true;
    config.general.respect_gitignore = false;

    config
        .save_to_path(&path)
        .expect("config save should succeed");

    let saved = fs::read_to_string(&path).expect("config file should exist");
    assert!(saved.contains("show_hidden = true"));
    assert!(saved.contains("respect_gitignore = false"));

    let loaded = Config::load_from_path(&path).expect("config load should succeed");
    assert!(loaded.general.show_hidden);
    assert!(!loaded.general.respect_gitignore);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn config_load_fills_missing_preview_mermaid_timeout_from_defaults() {
    let root = make_temp_dir("grove-config-missing-mermaid-timeout");
    let path = root.join("config.toml");
    fs::write(
        &path,
        r#"[general]
show_hidden = false
respect_gitignore = true
sort_by = "name"
theme = "dark"

[layout]
split_ratio = 0.4

[preview]
syntax_highlight = true
highlight_max_bytes = 1048576
raw_text_max_bytes = 4194304
binary_sniff_bytes = 8192
word_wrap = true
line_numbers = true

[git]
show_status = true
refresh_debounce_ms = 200

[injection.ai]
append_newline = false
batch_separator = "newline"
multiline_transport = "typed"
warn_line_count = 300

[injection.editor]
mode = "local_process"
command = "code"
args = ["-g", "{{path}}:{{line}}"]

[watcher]
debounce_ms = 125
highlight_changes_ms = 5000
poll_fallback = true

[bookmarks]
pins = []
"#,
    )
    .expect("legacy config should be written");

    let loaded = Config::load_from_path(&path).expect("legacy config should still load");
    assert_eq!(loaded.preview.mermaid_render_timeout_ms, 5_000);
    assert_eq!(loaded.preview.mermaid_command, None);
    assert_eq!(loaded.preview.image_preview_max_bytes, 20 * 1_048_576);
    assert_eq!(loaded.preview.image_preview_max_pixels, 16_777_216);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

fn make_temp_dir(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{label}-{nonce}"));
    fs::create_dir_all(&root).expect("temp root should be created");
    root
}
