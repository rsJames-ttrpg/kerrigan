/// Shared model types for the db layer.
///
/// These structs are the plain data types returned by db query functions.
/// They live here so multiple backend implementations can share them.
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

// ── Status enums ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for JobRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for JobRunStatus {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("invalid job run status: {other}")),
        }
    }
}

impl JobRunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => Err(format!("invalid task status: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HatcheryStatus {
    Online,
    Degraded,
    Offline,
}

impl fmt::Display for HatcheryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Degraded => write!(f, "degraded"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

impl FromStr for HatcheryStatus {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "online" => Ok(Self::Online),
            "degraded" => Ok(Self::Degraded),
            "offline" => Ok(Self::Offline),
            other => Err(format!("invalid hatchery status: {other}")),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub embedding_model: String,
    pub source: String,
    pub tags: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize)]
pub struct MemorySearchResult {
    pub memory: Memory,
    pub distance: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JobDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JobRun {
    pub id: String,
    pub definition_id: String,
    pub parent_id: Option<String>,
    pub status: JobRunStatus,
    pub triggered_by: String,
    pub config_overrides: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Task {
    pub id: String,
    pub run_id: Option<String>,
    pub subject: String,
    pub status: TaskStatus,
    pub assigned_to: Option<String>,
    pub output: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Decision {
    pub id: String,
    pub agent: String,
    pub context: String,
    pub decision: String,
    pub reasoning: String,
    pub tags: Vec<String>,
    pub run_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub enum ArtifactType {
    #[default]
    Generic,
    Conversation,
    Session,
    EvolutionReport,
}

impl fmt::Display for ArtifactType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generic => write!(f, "generic"),
            Self::Conversation => write!(f, "conversation"),
            Self::Session => write!(f, "session"),
            Self::EvolutionReport => write!(f, "evolution-report"),
        }
    }
}

impl FromStr for ArtifactType {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "generic" => Ok(Self::Generic),
            "conversation" => Ok(Self::Conversation),
            "session" => Ok(Self::Session),
            "evolution-report" => Ok(Self::EvolutionReport),
            other => Err(format!("invalid artifact type: {other}")),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ArtifactMetadata {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub run_id: Option<String>,
    pub artifact_type: ArtifactType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Hatchery {
    pub id: String,
    pub name: String,
    pub status: HatcheryStatus,
    pub capabilities: serde_json::Value,
    pub max_concurrency: i32,
    pub active_drones: i32,
    pub last_heartbeat_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    GithubPat,
    /// Preserves unknown credential types from the database so they survive
    /// round-tripping and can be grouped correctly in match queries.
    #[serde(untagged)]
    Other(String),
}

impl fmt::Display for CredentialType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GithubPat => write!(f, "github_pat"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

impl FromStr for CredentialType {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "github_pat" => Ok(Self::GithubPat),
            other => Ok(Self::Other(other.to_string())),
        }
    }
}

impl CredentialType {
    /// Map this credential type to the secrets key injected into job config.
    /// Returns `None` for unknown/unsupported types.
    pub fn secrets_key(&self) -> Option<&'static str> {
        match self {
            Self::GithubPat => Some("github_pat"),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Credential {
    pub id: String,
    pub pattern: String,
    pub credential_type: CredentialType,
    #[serde(skip_serializing)]
    pub secret: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hatchery_status_roundtrip() {
        for s in ["online", "degraded", "offline"] {
            let status: HatcheryStatus = s.parse().unwrap();
            assert_eq!(status.to_string(), s);
        }
    }

    #[test]
    fn test_hatchery_status_invalid() {
        assert!("bogus".parse::<HatcheryStatus>().is_err());
    }
}
