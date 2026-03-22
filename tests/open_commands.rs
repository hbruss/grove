use std::path::Path;

use grove::config::Config;
use grove::open::{
    EditorOpenRequest, ResolvedEditorOpen, resolve_editor_command,
    resolve_editor_command_from_value, resolve_editor_open, resolve_external_open_command,
    resolve_reveal_command,
};

#[test]
fn editor_resolution_prefers_explicit_config_command() {
    let mut config = Config::default();
    config.injection.editor.command = "hx".to_string();
    config.injection.editor.args = vec!["{{path}}".to_string()];

    let command = resolve_editor_command(&config, Path::new("/tmp/file.txt"));
    assert_eq!(command.program, "hx");
    assert_eq!(command.args, vec!["/tmp/file.txt"]);
}

#[test]
fn editor_resolution_falls_back_to_editor_env_when_config_uses_placeholder_default() {
    let command = resolve_editor_command_from_value(Path::new("/tmp/file.txt"), Some("micro"))
        .expect("editor env should resolve to a launch command");
    assert_eq!(command.program, "micro");
    assert_eq!(command.args, vec!["/tmp/file.txt"]);
}

#[test]
fn editor_resolution_supports_quoted_editor_arguments() {
    let command = resolve_editor_command_from_value(
        Path::new("/tmp/file.txt"),
        Some("nvim -c \"set number\""),
    )
    .expect("quoted editor args should resolve to a launch command");
    assert_eq!(command.program, "nvim");
    assert_eq!(command.args, vec!["-c", "set number", "/tmp/file.txt"]);
}

#[test]
fn editor_resolution_falls_back_to_micro_without_env() {
    let config = Config::default();

    let command = resolve_editor_command(&config, Path::new("/tmp/file.txt"));
    assert_eq!(command.program, "micro");
    assert_eq!(command.args, vec!["/tmp/file.txt"]);
}

#[test]
fn external_open_resolution_uses_platform_default_opener() {
    let command = resolve_external_open_command(Path::new("/tmp/index.html"));

    #[cfg(target_os = "macos")]
    assert_eq!(command.program, "open");

    #[cfg(target_os = "linux")]
    assert_eq!(command.program, "xdg-open");

    #[cfg(target_os = "windows")]
    assert_eq!(command.program, "cmd");
}

#[test]
fn editor_open_resolution_interpolates_requested_line_for_local_process() {
    let mut config = Config::default();
    config.injection.editor.command = "hx".to_string();
    config.injection.editor.args = vec!["{{path}}:{{line}}".to_string()];

    let resolved =
        resolve_editor_open(&config, &EditorOpenRequest::new("/tmp/file.txt".into(), 42));
    assert_eq!(
        resolved,
        ResolvedEditorOpen::LocalProcess(grove::open::LaunchCommand {
            program: "hx".to_string(),
            args: vec!["/tmp/file.txt:42".to_string()],
        })
    );
}

#[test]
fn editor_open_resolution_builds_shell_target_command_line() {
    let mut config = Config::default();
    config.injection.editor.mode = grove::config::EditorMode::ShellTarget;
    config.injection.editor.command = "hx".to_string();
    config.injection.editor.args = vec!["{{path}}:{{line}}".to_string()];

    let resolved = resolve_editor_open(
        &config,
        &EditorOpenRequest::new("/tmp/file with space.txt".into(), 7),
    );
    assert_eq!(
        resolved,
        ResolvedEditorOpen::ShellTarget("'hx' '/tmp/file with space.txt:7'".to_string())
    );
}

#[test]
fn reveal_resolution_uses_platform_specific_reveal_command() {
    let command = resolve_reveal_command(Path::new("/tmp/index.html"));

    #[cfg(target_os = "macos")]
    {
        assert_eq!(command.program, "open");
        assert_eq!(command.args, vec!["-R", "/tmp/index.html"]);
    }

    #[cfg(target_os = "linux")]
    {
        assert_eq!(command.program, "xdg-open");
        assert_eq!(command.args, vec!["/tmp"]);
    }

    #[cfg(target_os = "windows")]
    {
        assert_eq!(command.program, "explorer");
    }
}
