use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    Process,
    Powershell,
    Docker,
    Android,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub kind: ProjectKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub accent: String,
    pub cover: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "empty_config")]
    pub config: Value,
}
fn empty_config() -> Value {
    serde_json::json!({})
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Setup,
    Stopped,
    Starting,
    Ready,
    External,
    Stopping,
    Error,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatus {
    pub id: String,
    pub phase: Phase,
    pub owned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}
impl ProjectStatus {
    pub fn new(id: &str, phase: Phase, owned: bool, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            phase,
            owned,
            message: Some(message.into()),
            pid: None,
            serial: None,
            started_at: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub close_to_tray: bool,
    #[serde(default = "yes")]
    pub auto_open_browser: bool,
    #[serde(default = "light")]
    pub theme: String,
    #[serde(default)]
    pub live_actions_enabled: bool,
}
fn yes() -> bool {
    true
}
fn light() -> String {
    "light".into()
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            auto_open_browser: true,
            theme: light(),
            live_actions_enabled: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileValidation {
    pub complete: bool,
    pub issues: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticState {
    Checking,
    Unknown,
    Ready,
    Error,
    NotApplicable,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub id: String,
    pub label: String,
    pub state: DiagnosticState,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDiagnostics {
    pub id: String,
    pub checked_at: String,
    pub checks: Vec<DiagnosticCheck>,
    pub technologies: Vec<String>,
    pub can_stop: bool,
    pub can_restart: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_reason: Option<String>,
}
