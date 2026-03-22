pub mod content;

use serde::{Deserialize, Serialize};

use crate::preview::model::{SearchGeneration, SearchPayload};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
    WholeRepo,
    OpenFiles,
}

impl Default for SearchScope {
    fn default() -> Self {
        Self::WholeRepo
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchRequest {
    pub query: String,
    pub scope: SearchScope,
    pub max_results: usize,
    pub include_context_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchResponse {
    pub generation: SearchGeneration,
    pub payload: SearchPayload,
}
