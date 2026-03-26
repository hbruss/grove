use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRole {
    Grove,
    Ai,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendTarget {
    Role(TargetRole),
    SessionId(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct SessionLocationHint {
    pub window_id: Option<String>,
    pub tab_id: Option<String>,
    pub window_title: Option<String>,
    pub tab_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub role: Option<TargetRole>,
    pub job_name: Option<String>,
    pub command_line: Option<String>,
    pub cwd: Option<String>,
    pub location_hint: Option<SessionLocationHint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionSource {
    #[default]
    SameTab,
    SameWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetResolution {
    pub ai_target_session_id: Option<String>,
    pub editor_target_session_id: Option<String>,
    pub source: ResolutionSource,
}

impl Default for TargetResolution {
    fn default() -> Self {
        Self {
            ai_target_session_id: None,
            editor_target_session_id: None,
            source: ResolutionSource::SameTab,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeCommand {
    Ping,
    ListSessions {
        instance_id: String,
    },
    SetRole {
        session_id: String,
        role: TargetRole,
    },
    ClearRole {
        session_id: String,
    },
    ResolveTargets {
        instance_id: String,
    },
    SendText {
        instance_id: String,
        target: SendTarget,
        text: String,
        append_newline: bool,
    },
    GetSessionSnapshot {
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeResponse {
    Pong,
    SessionList(Vec<SessionSummary>),
    TargetsResolved(TargetResolution),
    ManualSelectionRequired { role: TargetRole },
    TargetSessionUnavailable { session_id: String },
    SendOk { target_session_id: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRequestEnvelope {
    pub request_id: String,
    pub command: BridgeCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeResponseEnvelope {
    pub request_id: String,
    pub response: BridgeResponse,
}
