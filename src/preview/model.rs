use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::state::ContextMode;

#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct PreviewGeneration(pub u64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PreviewPresentation {
    #[default]
    Standard,
    Diff,
    ImagePending,
    ImageInline,
    ImageSummary,
    MermaidPending,
    MermaidAscii,
    MermaidImage,
    MermaidRawSource,
}

#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct SearchGeneration(pub u64);

#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct GitGeneration(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PreviewMetadataItem {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PreviewHeader {
    pub path: Option<String>,
    pub metadata: Vec<PreviewMetadataItem>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MermaidSourceKind {
    #[default]
    NativeFile,
    MarkdownFence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MermaidSource {
    pub kind: MermaidSourceKind,
    pub block_index: Option<usize>,
    pub total_blocks: usize,
    pub label: String,
    pub raw_source: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MermaidDisplay {
    #[default]
    Pending,
    Ascii,
    Image,
    RawSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MermaidPreview {
    pub source: MermaidSource,
    pub display: MermaidDisplay,
    pub status: String,
    pub body_lines: Vec<String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageDisplay {
    #[default]
    Pending,
    Inline,
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ImagePreview {
    pub display: ImageDisplay,
    pub status: String,
    pub format_label: String,
    pub dimensions: Option<(u32, u32)>,
    pub body_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PreviewPayload {
    pub title: String,
    pub header: PreviewHeader,
    pub lines: Vec<String>,
    pub markdown: Option<String>,
    pub image: Option<ImagePreview>,
    pub mermaid: Option<MermaidPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PreviewSource {
    pub rel_path: Option<PathBuf>,
    pub context_mode: ContextMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchHit {
    pub path: String,
    pub line: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchPayload {
    pub query: String,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitPayload {
    pub summary: String,
}
