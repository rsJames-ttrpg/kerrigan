use async_trait::async_trait;

use crate::harness::QueenChannel;
use crate::protocol::{DroneEnvironment, DroneOutput, JobSpec};

/// Trait that every drone binary implements.
///
/// The harness calls these methods in order:
/// 1. `setup` — create isolated environment (temp dirs, extract config, clone repo)
/// 2. `execute` — run the agent CLI, communicate with Queen via channel
/// 3. `teardown` — clean up temp dirs and child processes
#[async_trait]
pub trait DroneRunner: Send + Sync {
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment>;

    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &mut QueenChannel,
    ) -> anyhow::Result<DroneOutput>;

    async fn teardown(&self, env: &DroneEnvironment);
}
