use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use grove::app::App;

#[test]
fn app_save_config_requires_an_explicit_config_path() {
    let config_home = make_temp_dir("grove-app-config-home");
    let expected_config_path = config_home.join("grove").join("config.toml");
    let mut app = App::default();
    app.config
        .bookmarks
        .pins
        .push(PathBuf::from("/tmp/example-root"));

    let result = with_test_config_home(&config_home, || app.save_config());

    assert!(
        result.is_err(),
        "save_config should fail without config_path"
    );
    assert!(
        !expected_config_path.exists(),
        "save_config should not write the default config path implicitly"
    );

    fs::remove_dir_all(config_home).expect("temp config home should be removed");
}

fn with_test_config_home<T>(config_home: &std::path::Path, operation: impl FnOnce() -> T) -> T {
    let original_config_home = std::env::var_os("XDG_CONFIG_HOME");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_home);
    }

    let result = operation();

    unsafe {
        match original_config_home {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    result
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
