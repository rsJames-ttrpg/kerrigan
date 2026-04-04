use async_trait::async_trait;
use drone_sdk::{
    harness::QueenChannel,
    protocol::{DroneEnvironment, DroneOutput, GitRefs, JobSpec},
    runner::DroneRunner,
};
use serde_json::json;

pub struct NativeDrone;

#[async_trait]
impl DroneRunner for NativeDrone {
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment> {
        tracing::info!(run_id = %job.job_run_id, "Setting up native drone");
        let home = std::path::PathBuf::from(format!("/tmp/drone-{}", job.job_run_id));
        tokio::fs::create_dir_all(&home).await?;
        Ok(DroneEnvironment {
            home: home.clone(),
            workspace: home.join("workspace"),
        })
    }

    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &mut QueenChannel,
    ) -> anyhow::Result<DroneOutput> {
        channel.progress("started", "native drone placeholder")?;
        tracing::info!("Native drone execute — placeholder");
        Ok(DroneOutput {
            exit_code: 0,
            conversation: json!({}),
            artifacts: vec![],
            git_refs: GitRefs {
                branch: None,
                pr_url: None,
                pr_required: false,
            },
            session_jsonl_gz: None,
        })
    }

    async fn teardown(&self, env: &DroneEnvironment) {
        let _ = tokio::fs::remove_dir_all(&env.home).await;
    }
}
