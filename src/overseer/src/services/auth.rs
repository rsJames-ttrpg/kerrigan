use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory store for pending auth codes.
///
/// Flow: drone needs auth → Queen notifies Overseer → user POSTs code → Queen polls and retrieves it.
/// Codes are consumed on retrieval (single-use).
pub struct AuthService {
    pending: Mutex<HashMap<String, String>>,
}

impl AuthService {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Store an auth code for a job run. Overwrites any previous code.
    pub fn submit_code(&self, job_run_id: &str, code: &str) {
        self.pending
            .lock()
            .unwrap()
            .insert(job_run_id.to_string(), code.to_string());
    }

    /// Retrieve and consume an auth code for a job run. Returns None if no code has been submitted.
    pub fn take_code(&self, job_run_id: &str) -> Option<String> {
        self.pending.lock().unwrap().remove(job_run_id)
    }
}
