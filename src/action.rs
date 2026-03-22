use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    MoveUp,
    MoveDown,
    FocusNextPanel,
    ExpandDirectory,
    CollapseDirectory,
    ToggleSelect,
    FocusPathFilter,
    OpenContentSearch,
    SetContextModePreview,
    SetContextModeDiff,
    SetContextModeBlame,
    SetContextModeInfo,
    SendPath,
    SendRelativePath,
    SendRef,
    SendContents,
    SendDiff,
    SendSnippet,
    CopyRelativePath,
    CopyAbsolutePath,
    StageSelectedPath,
    UnstageSelectedPath,
    NewFile,
    NewDirectory,
    Rename,
    Duplicate,
    Move,
    Trash,
    OpenInEditor,
    OpenExternally,
    RevealInFinder,
    NewTab,
    CloseTab,
    PinBookmark,
    UnpinBookmark,
    SetAiTarget,
    SetEditorTarget,
    ClearAiTarget,
    ClearEditorTarget,
    ToggleHiddenFiles,
    ToggleGitignore,
    OpenCommandPalette,
    OpenContextMenu,
    OpenDialog,
    CloseOverlay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDescriptor {
    pub action: Action,
    pub label: String,
    pub subtitle: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootWorkflowContract {
    OpenSelectedDirectoryAsRootTab,
    ToggleActiveRootBookmark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSurfaceContract {
    UnifiedPalette,
}
