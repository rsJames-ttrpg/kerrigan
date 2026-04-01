// Protocol types for Queen <-> Drone JSON-line communication

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Messages sent from the Queen to a Drone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum QueenMessage {
    Job(JobSpec),
    AuthResponse(AuthResponse),
    Cancel {},
}

/// Messages sent from a Drone back to the Queen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DroneMessage {
    AuthRequest(AuthRequest),
    Progress(Progress),
    Result(DroneOutput),
    Error(DroneError),
}

/// Initial job specification sent from Queen to Drone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub job_run_id: String,
    pub repo_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub task: String,
    pub config: Value,
}

/// Human approval/denial response for an auth request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Request from Drone for human to visit a URL (e.g. OAuth).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub url: String,
    pub message: String,
}

/// Status update from Drone to Queen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Final output from a Drone on successful completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroneOutput {
    pub exit_code: i32,
    pub conversation: Value,
    pub artifacts: Vec<String>,
    pub git_refs: GitRefs,
}

/// Git references produced by a completed Drone job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
}

/// Fatal error reported by a Drone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroneError {
    pub message: String,
}

/// Internal (non-serialized) environment paths used by the Drone runner.
#[derive(Debug, Clone)]
pub struct DroneEnvironment {
    pub home: PathBuf,
    pub workspace: PathBuf,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_queen_job_message_roundtrip() {
        let spec = JobSpec {
            job_run_id: "run-42".to_string(),
            repo_url: "https://github.com/example/repo".to_string(),
            branch: Some("main".to_string()),
            task: "fix the bug".to_string(),
            config: json!({"timeout_secs": 300}),
        };
        let msg = QueenMessage::Job(spec);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: QueenMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            QueenMessage::Job(s) => {
                assert_eq!(s.job_run_id, "run-42");
                assert_eq!(s.repo_url, "https://github.com/example/repo");
                assert_eq!(s.branch.as_deref(), Some("main"));
                assert_eq!(s.task, "fix the bug");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_queen_cancel_message() {
        let msg = QueenMessage::Cancel {};
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: QueenMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded, QueenMessage::Cancel {}));
        // Verify the JSON contains the right type tag
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "cancel");
    }

    #[test]
    fn test_drone_result_message_roundtrip() {
        let output = DroneOutput {
            exit_code: 0,
            conversation: json!([{"role": "user", "content": "hello"}]),
            artifacts: vec!["output.patch".to_string()],
            git_refs: GitRefs {
                branch: Some("feat/fix-bug".to_string()),
                pr_url: Some("https://github.com/example/repo/pull/1".to_string()),
            },
        };
        let msg = DroneMessage::Result(output);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Result(o) => {
                assert_eq!(o.exit_code, 0);
                assert_eq!(o.artifacts, vec!["output.patch"]);
                assert_eq!(o.git_refs.branch.as_deref(), Some("feat/fix-bug"));
                assert_eq!(
                    o.git_refs.pr_url.as_deref(),
                    Some("https://github.com/example/repo/pull/1")
                );
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_drone_auth_request() {
        let auth = AuthRequest {
            url: "https://auth.example.com/oauth".to_string(),
            message: "Please approve access".to_string(),
        };
        let msg = DroneMessage::AuthRequest(auth);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::AuthRequest(a) => {
                assert_eq!(a.url, "https://auth.example.com/oauth");
                assert_eq!(a.message, "Please approve access");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "auth_request");
    }

    #[test]
    fn test_drone_progress() {
        let progress = Progress {
            status: "running".to_string(),
            detail: Some("compiling sources".to_string()),
        };
        let msg = DroneMessage::Progress(progress);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Progress(p) => {
                assert_eq!(p.status, "running");
                assert_eq!(p.detail.as_deref(), Some("compiling sources"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_drone_error() {
        let err = DroneError {
            message: "workspace clone failed".to_string(),
        };
        let msg = DroneMessage::Error(err);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Error(e) => {
                assert_eq!(e.message, "workspace clone failed");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "error");
    }
}
