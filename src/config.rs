use std::fs;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    Name,
}

impl Default for SortMode {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    Dark,
}

impl Default for ThemeName {
    fn default() -> Self {
        Self::Dark
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MultilineTransport {
    Typed,
    BracketedPaste,
}

impl Default for MultilineTransport {
    fn default() -> Self {
        Self::Typed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditorMode {
    LocalProcess,
    ShellTarget,
}

impl Default for EditorMode {
    fn default() -> Self {
        Self::LocalProcess
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub layout: LayoutConfig,
    pub preview: PreviewConfig,
    pub git: GitConfig,
    pub injection: InjectionConfig,
    pub watcher: WatcherConfig,
    pub bookmarks: BookmarksConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub show_hidden: bool,
    pub respect_gitignore: bool,
    pub sort_by: SortMode,
    pub theme: ThemeName,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            show_hidden: false,
            respect_gitignore: true,
            sort_by: SortMode::Name,
            theme: ThemeName::Dark,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub split_ratio: f32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self { split_ratio: 0.40 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PreviewConfig {
    pub syntax_highlight: bool,
    pub highlight_max_bytes: usize,
    pub raw_text_max_bytes: usize,
    pub binary_sniff_bytes: usize,
    pub image_preview_max_bytes: usize,
    pub image_preview_max_pixels: u64,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub mermaid_command: Option<String>,
    pub mermaid_render_timeout_ms: u64,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            syntax_highlight: true,
            highlight_max_bytes: 1_048_576,
            raw_text_max_bytes: 4_194_304,
            binary_sniff_bytes: 8_192,
            image_preview_max_bytes: 20 * 1_048_576,
            image_preview_max_pixels: 16_777_216,
            word_wrap: true,
            line_numbers: true,
            mermaid_command: None,
            mermaid_render_timeout_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    pub show_status: bool,
    pub refresh_debounce_ms: u64,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            show_status: true,
            refresh_debounce_ms: 200,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InjectionConfig {
    pub ai: AiInjectionConfig,
    pub editor: EditorInjectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiInjectionConfig {
    pub append_newline: bool,
    pub batch_separator: String,
    pub multiline_transport: MultilineTransport,
    pub warn_line_count: usize,
}

impl Default for AiInjectionConfig {
    fn default() -> Self {
        Self {
            append_newline: false,
            batch_separator: "newline".to_string(),
            multiline_transport: MultilineTransport::Typed,
            warn_line_count: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorInjectionConfig {
    pub mode: EditorMode,
    pub command: String,
    pub args: Vec<String>,
}

impl Default for EditorInjectionConfig {
    fn default() -> Self {
        Self {
            mode: EditorMode::LocalProcess,
            command: "code".to_string(),
            args: vec!["-g".to_string(), "{{path}}:{{line}}".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherConfig {
    pub debounce_ms: u64,
    pub highlight_changes_ms: u64,
    pub poll_fallback: bool,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 125,
            highlight_changes_ms: 5_000,
            poll_fallback: true,
        }
    }
}

impl WatcherConfig {
    pub fn debounce_duration(&self) -> Duration {
        Duration::from_millis(self.debounce_ms)
    }

    pub fn highlight_changes_duration(&self) -> Duration {
        Duration::from_millis(self.highlight_changes_ms)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BookmarksConfig {
    pub pins: Vec<PathBuf>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = crate::storage::config_path()?;
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents)
                .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()).into()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = crate::storage::config_path()?;
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let serialized =
            toml::to_string_pretty(self).map_err(|err| Error::other(err.to_string()))?;
        fs::write(path, serialized)?;
        Ok(())
    }
}
