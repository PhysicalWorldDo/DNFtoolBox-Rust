use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fmt, path::PathBuf};

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}
impl CoreError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: matches!(code, "NETWORK" | "FILE_IN_USE" | "BUSY"),
            operation_id: None,
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", message)
    }
    pub fn cancelled() -> Self {
        Self::new("CANCELLED", "任务已取消")
    }
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CoreError {}
impl From<std::io::Error> for CoreError {
    fn from(error: std::io::Error) -> Self {
        let code = match error.raw_os_error() {
            Some(32 | 33) => "FILE_IN_USE",
            Some(112) => "INSUFFICIENT_SPACE",
            _ => "IO",
        };
        Self::new(code, error.to_string())
    }
}
impl From<serde_json::Error> for CoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::invalid(error.to_string())
    }
}
impl From<zip::result::ZipError> for CoreError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::invalid(format!("无效 ZIP：{error}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CoreError>,
}
impl Response {
    pub fn success(id: impl Into<String>, result: Value) -> Self {
        Self {
            v: 1,
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }
    pub fn failure(id: impl Into<String>, error: CoreError) -> Self {
        Self {
            v: 1,
            id: id.into(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub v: u32,
    pub session_id: String,
    pub seq: u64,
    pub event: String,
    pub data: Value,
}
#[derive(Debug, Clone)]
pub enum NativeAction {
    OpenPath(PathBuf),
    OpenUrl(String),
    CopyText(String),
}

fn one() -> u32 {
    1
}
fn stable() -> String {
    "stable".into()
}
fn category() -> String {
    "其他工具".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolboxUpdate {
    #[serde(default)]
    pub latest_version: String,
    #[serde(default)]
    pub min_supported_version: String,
    #[serde(default)]
    pub release_url: String,
    #[serde(default)]
    pub package_url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub changelog: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexTool {
    pub id: String,
    pub name: String,
    #[serde(default = "category")]
    pub category: String,
    pub manifest_url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Index {
    #[serde(default = "one")]
    pub schema_version: u32,
    #[serde(default)]
    pub toolbox: ToolboxUpdate,
    #[serde(default)]
    pub tools: Vec<IndexTool>,
}
impl Default for Index {
    fn default() -> Self {
        Self {
            schema_version: 1,
            toolbox: ToolboxUpdate::default(),
            tools: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolVersion {
    pub version: String,
    #[serde(default = "stable")]
    pub channel: String,
    #[serde(default)]
    pub package_url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub release_date: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub changelog: Vec<String>,
    #[serde(default)]
    pub min_toolbox_version: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    #[serde(default = "one")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default = "category")]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub icon_source: String,
    pub entry: String,
    #[serde(default)]
    pub need_admin: bool,
    #[serde(default)]
    pub project_url: String,
    #[serde(default)]
    pub latest: BTreeMap<String, String>,
    #[serde(default)]
    pub versions: Vec<ToolVersion>,
    #[serde(default)]
    pub blocked_versions: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default = "stable")]
    pub channel: String,
    pub entry: String,
    #[serde(default)]
    pub installed_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Operation {
    pub id: String,
    pub kind: String,
    pub tool_id: Option<String>,
    pub version: Option<String>,
    pub state: String,
    pub stage: String,
    pub done_bytes: u64,
    pub total_bytes: Option<u64>,
    pub can_cancel: bool,
    pub error: Option<CoreError>,
    pub children: Vec<Value>,
    pub result: Option<Value>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub request: Value,
}
impl Operation {
    pub fn active(&self) -> bool {
        matches!(self.state.as_str(), "queued" | "running")
    }
}
pub(crate) fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}
