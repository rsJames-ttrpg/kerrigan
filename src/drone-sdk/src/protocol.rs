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
    Event(DroneEvent),
}

/// Rich structured events emitted by a Drone during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DroneEvent {
    ToolUse {
        name: String,
        duration_ms: u64,
        tokens_used: u32,
    },
    Checkpoint {
        artifact_id: String,
        tokens_before: u32,
        tokens_after: u32,
    },
    TaskStarted {
        task_id: String,
        description: String,
    },
    TaskCompleted {
        task_id: String,
        description: String,
    },
    StageTransition {
        from: String,
        to: String,
    },
    SubAgentSpawned {
        agent_id: String,
        task: String,
    },
    SubAgentCompleted {
        agent_id: String,
        success: bool,
    },
    GitCommit {
        sha: String,
        message: String,
    },
    GitPrCreated {
        url: String,
    },
    TestResults {
        passed: u32,
        failed: u32,
        skipped: u32,
    },
    TokenUsage {
        input: u32,
        output: u32,
        cache_read: u32,
        total_cost_usd: Option<f64>,
    },
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
    /// Full session JSONL, gzipped and base64-encoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_jsonl_gz: Option<String>,
}

/// Git references produced by a completed Drone job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// Whether this job type requires a PR for success. Defaults to true.
    #[serde(default = "default_true")]
    pub pr_required: bool,
}

fn default_true() -> bool {
    true
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
                pr_required: true,
            },
            session_jsonl_gz: None,
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
    fn test_git_refs_pr_required_defaults_true() {
        // Simulate deserializing a GitRefs from an older drone that doesn't send pr_required
        let json = r#"{"branch":"feat/x","pr_url":"https://github.com/org/repo/pull/1"}"#;
        let refs: GitRefs = serde_json::from_str(json).unwrap();
        assert!(
            refs.pr_required,
            "pr_required should default to true for backwards compat"
        );
    }

    #[test]
    fn test_git_refs_pr_required_false_roundtrip() {
        let refs = GitRefs {
            branch: Some("evolve/analysis".to_string()),
            pr_url: None,
            pr_required: false,
        };
        let json = serde_json::to_string(&refs).unwrap();
        let decoded: GitRefs = serde_json::from_str(&json).unwrap();
        assert!(!decoded.pr_required);
        assert!(decoded.pr_url.is_none());
    }

    #[test]
    fn test_drone_result_no_pr_not_required() {
        let output = DroneOutput {
            exit_code: 0,
            conversation: json!({}),
            artifacts: vec![],
            git_refs: GitRefs {
                branch: None,
                pr_url: None,
                pr_required: false,
            },
            session_jsonl_gz: None,
        };
        let msg = DroneMessage::Result(output);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Result(o) => {
                assert_eq!(o.exit_code, 0);
                assert!(!o.git_refs.pr_required);
                assert!(o.git_refs.pr_url.is_none());
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
    fn test_drone_event_tool_use_roundtrip() {
        let event = DroneEvent::ToolUse {
            name: "bash".to_string(),
            duration_ms: 1500,
            tokens_used: 42,
        };
        let msg = DroneMessage::Event(event);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Event(DroneEvent::ToolUse {
                name,
                duration_ms,
                tokens_used,
            }) => {
                assert_eq!(name, "bash");
                assert_eq!(duration_ms, 1500);
                assert_eq!(tokens_used, 42);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "event");
    }

    #[test]
    fn test_drone_event_token_usage_roundtrip() {
        let event = DroneEvent::TokenUsage {
            input: 1000,
            output: 500,
            cache_read: 200,
            total_cost_usd: Some(0.015),
        };
        let msg = DroneMessage::Event(event);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Event(DroneEvent::TokenUsage {
                input,
                output,
                cache_read,
                total_cost_usd,
            }) => {
                assert_eq!(input, 1000);
                assert_eq!(output, 500);
                assert_eq!(cache_read, 200);
                assert_eq!(total_cost_usd, Some(0.015));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_drone_event_checkpoint_roundtrip() {
        let event = DroneEvent::Checkpoint {
            artifact_id: "chk-123".to_string(),
            tokens_before: 50000,
            tokens_after: 10000,
        };
        let msg = DroneMessage::Event(event);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Event(DroneEvent::Checkpoint {
                artifact_id,
                tokens_before,
                tokens_after,
            }) => {
                assert_eq!(artifact_id, "chk-123");
                assert_eq!(tokens_before, 50000);
                assert_eq!(tokens_after, 10000);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_drone_event_test_results_roundtrip() {
        let event = DroneEvent::TestResults {
            passed: 42,
            failed: 1,
            skipped: 3,
        };
        let msg = DroneMessage::Event(event);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Event(DroneEvent::TestResults {
                passed,
                failed,
                skipped,
            }) => {
                assert_eq!(passed, 42);
                assert_eq!(failed, 1);
                assert_eq!(skipped, 3);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_drone_event_git_commit_roundtrip() {
        let event = DroneEvent::GitCommit {
            sha: "abc1234".to_string(),
            message: "fix: resolve auth issue".to_string(),
        };
        let msg = DroneMessage::Event(event);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Event(DroneEvent::GitCommit { sha, message }) => {
                assert_eq!(sha, "abc1234");
                assert_eq!(message, "fix: resolve auth issue");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_drone_event_stage_transition_roundtrip() {
        let event = DroneEvent::StageTransition {
            from: "plan".to_string(),
            to: "implement".to_string(),
        };
        let msg = DroneMessage::Event(event);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: DroneMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            DroneMessage::Event(DroneEvent::StageTransition { from, to }) => {
                assert_eq!(from, "plan");
                assert_eq!(to, "implement");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_old_variants_still_deserialize_with_event_added() {
        // Verify that adding Event variant doesn't break existing message parsing
        let progress_json =
            r#"{"type":"progress","payload":{"status":"running","detail":"compiling"}}"#;
        let decoded: DroneMessage = serde_json::from_str(progress_json).unwrap();
        assert!(matches!(decoded, DroneMessage::Progress(_)));

        let error_json = r#"{"type":"error","payload":{"message":"something broke"}}"#;
        let decoded: DroneMessage = serde_json::from_str(error_json).unwrap();
        assert!(matches!(decoded, DroneMessage::Error(_)));

        let auth_json = r#"{"type":"auth_request","payload":{"url":"https://example.com","message":"approve"}}"#;
        let decoded: DroneMessage = serde_json::from_str(auth_json).unwrap();
        assert!(matches!(decoded, DroneMessage::AuthRequest(_)));
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
