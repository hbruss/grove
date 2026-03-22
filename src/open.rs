use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Config, EditorInjectionConfig, EditorMode};
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorOpenRequest {
    pub path: PathBuf,
    pub line: usize,
}

impl EditorOpenRequest {
    pub fn new(path: PathBuf, line: usize) -> Self {
        Self {
            path,
            line: line.max(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedEditorOpen {
    LocalProcess(LaunchCommand),
    ShellTarget(String),
}

pub fn resolve_editor_command(config: &Config, path: &Path) -> LaunchCommand {
    resolve_editor_command_request(config, &EditorOpenRequest::new(path.to_path_buf(), 1))
}

pub fn resolve_editor_open(config: &Config, request: &EditorOpenRequest) -> ResolvedEditorOpen {
    match config.injection.editor.mode {
        EditorMode::LocalProcess => {
            ResolvedEditorOpen::LocalProcess(resolve_editor_command_request(config, request))
        }
        EditorMode::ShellTarget => {
            ResolvedEditorOpen::ShellTarget(resolve_shell_target_command(config, request))
        }
    }
}

fn resolve_editor_command_request(config: &Config, request: &EditorOpenRequest) -> LaunchCommand {
    resolve_editor_command_request_with_value(config, request, std::env::var("EDITOR").ok())
}

fn resolve_editor_command_request_with_value(
    config: &Config,
    request: &EditorOpenRequest,
    editor_value: Option<String>,
) -> LaunchCommand {
    resolve_configured_editor_command(config, request)
        .or_else(|| resolve_editor_command_from_value(&request.path, editor_value.as_deref()))
        .unwrap_or_else(|| LaunchCommand {
            program: "micro".to_string(),
            args: vec![request.path.display().to_string()],
        })
}

pub fn resolve_editor_command_from_value(
    path: &Path,
    editor_value: Option<&str>,
) -> Option<LaunchCommand> {
    let editor = editor_value?;
    let mut parts = shlex::split(editor)?;
    let program = parts.first()?.clone();
    let mut args = parts.drain(1..).collect::<Vec<_>>();
    args.push(path.display().to_string());
    Some(LaunchCommand { program, args })
}

pub fn resolve_external_open_command(path: &Path) -> LaunchCommand {
    #[cfg(target_os = "macos")]
    {
        return LaunchCommand {
            program: "open".to_string(),
            args: vec![path.display().to_string()],
        };
    }

    #[cfg(target_os = "linux")]
    {
        return LaunchCommand {
            program: "xdg-open".to_string(),
            args: vec![path.display().to_string()],
        };
    }

    #[cfg(target_os = "windows")]
    {
        return LaunchCommand {
            program: "cmd".to_string(),
            args: vec![
                "/C".to_string(),
                "start".to_string(),
                "".to_string(),
                path.display().to_string(),
            ],
        };
    }

    #[allow(unreachable_code)]
    LaunchCommand {
        program: "open".to_string(),
        args: vec![path.display().to_string()],
    }
}

pub fn resolve_reveal_command(path: &Path) -> LaunchCommand {
    #[cfg(target_os = "macos")]
    {
        return LaunchCommand {
            program: "open".to_string(),
            args: vec!["-R".to_string(), path.display().to_string()],
        };
    }

    #[cfg(target_os = "linux")]
    {
        let parent = path.parent().unwrap_or(path);
        return LaunchCommand {
            program: "xdg-open".to_string(),
            args: vec![parent.display().to_string()],
        };
    }

    #[cfg(target_os = "windows")]
    {
        return LaunchCommand {
            program: "explorer".to_string(),
            args: vec![format!("/select,{}", path.display())],
        };
    }

    #[allow(unreachable_code)]
    LaunchCommand {
        program: "open".to_string(),
        args: vec![path.display().to_string()],
    }
}

pub fn launch_blocking(command: &LaunchCommand) -> Result<()> {
    let status = Command::new(&command.program)
        .args(&command.args)
        .status()?;
    if status.success() {
        return Ok(());
    }

    Err(std::io::Error::other(format!(
        "command exited unsuccessfully: {} {}",
        command.program,
        command.args.join(" ")
    ))
    .into())
}

fn resolve_configured_editor_command(
    config: &Config,
    request: &EditorOpenRequest,
) -> Option<LaunchCommand> {
    let editor = &config.injection.editor;
    if !has_explicit_editor_command(editor) {
        return None;
    }

    Some(LaunchCommand {
        program: replace_placeholders(&editor.command, request),
        args: editor
            .args
            .iter()
            .map(|arg| replace_placeholders(arg, request))
            .collect(),
    })
}

fn has_explicit_editor_command(editor: &EditorInjectionConfig) -> bool {
    let command = editor.command.trim();
    !command.is_empty() && !uses_placeholder_editor_default(editor)
}

fn uses_placeholder_editor_default(editor: &EditorInjectionConfig) -> bool {
    editor.command == "code" && editor.args == ["-g", "{{path}}:{{line}}"]
}

fn replace_placeholders(template: &str, request: &EditorOpenRequest) -> String {
    template
        .replace("{{path}}", &request.path.display().to_string())
        .replace("{{line}}", &request.line.to_string())
}

fn resolve_shell_target_command(config: &Config, request: &EditorOpenRequest) -> String {
    let command = resolve_shell_target_launch_command(config, request);
    std::iter::once(command.program.as_str())
        .chain(command.args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn resolve_shell_target_launch_command(
    config: &Config,
    request: &EditorOpenRequest,
) -> LaunchCommand {
    resolve_shell_target_launch_command_with_value(config, request, std::env::var("EDITOR").ok())
}

fn resolve_shell_target_launch_command_with_value(
    config: &Config,
    request: &EditorOpenRequest,
    editor_value: Option<String>,
) -> LaunchCommand {
    resolve_configured_editor_command(config, request)
        .or_else(|| resolve_editor_command_from_value(&request.path, editor_value.as_deref()))
        .unwrap_or_else(|| LaunchCommand {
            program: "micro".to_string(),
            args: vec![request.path.display().to_string()],
        })
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EditorMode;

    #[test]
    fn shell_target_uses_editor_env_when_config_uses_placeholder_default() {
        let mut config = Config::default();
        config.injection.editor.mode = EditorMode::ShellTarget;

        let request = EditorOpenRequest::new("/tmp/file.txt".into(), 12);
        let command = resolve_shell_target_launch_command_with_value(
            &config,
            &request,
            Some("hx".to_string()),
        );

        assert_eq!(command.program, "hx");
        assert_eq!(command.args, vec!["/tmp/file.txt"]);
    }
}
