use std::path::PathBuf;

use crate::bridge::protocol::BridgeResponseEnvelope;
use crate::preview::model::{
    GitGeneration, GitPayload, PreviewGeneration, PreviewPayload, SearchGeneration, SearchPayload,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key(String),
    Mouse { x: u16, y: u16 },
    Resize { width: u16, height: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FsBatch {
    pub changed_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexBatch {
    pub indexed_paths: Vec<PathBuf>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeEvent {
    Connected,
    Disconnected,
    Response(BridgeResponseEnvelope),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerKind {
    UiTick,
    StatusTimeout,
    WatchDebounce,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    Input(InputEvent),
    Fs(FsBatch),
    Index(IndexBatch),
    PreviewReady(PreviewGeneration, Box<PreviewPayload>),
    SearchReady(SearchGeneration, SearchPayload),
    GitReady(GitGeneration, GitPayload),
    Bridge(BridgeEvent),
    Timer(TimerKind),
}
