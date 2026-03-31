use crate::protocol::*;
use crate::runner::DroneRunner;
use std::sync::{Arc, Mutex};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct QueenChannel {
    writer: Arc<Mutex<io::Stdout>>,
    reader: Arc<Mutex<BufReader<io::Stdin>>>,
}

impl QueenChannel {
    fn new() -> Self {
        Self {
            writer: Arc::new(Mutex::new(io::stdout())),
            reader: Arc::new(Mutex::new(BufReader::new(io::stdin()))),
        }
    }

    async fn send(&self, msg: &DroneMessage) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(line.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    async fn recv(&self) -> anyhow::Result<QueenMessage> {
        let mut line = String::new();
        let mut reader = self.reader.lock().unwrap();
        reader.read_line(&mut line).await?;
        let msg: QueenMessage = serde_json::from_str(line.trim())?;
        Ok(msg)
    }

    pub async fn request_auth(&self, url: &str, message: &str) -> anyhow::Result<bool> {
        self.send(&DroneMessage::AuthRequest(AuthRequest {
            url: url.to_string(),
            message: message.to_string(),
        }))
        .await?;

        let msg = self.recv().await?;
        match msg {
            QueenMessage::AuthResponse(resp) => Ok(resp.approved),
            QueenMessage::Cancel {} => anyhow::bail!("cancelled by queen"),
            _ => anyhow::bail!("unexpected message from queen: expected auth_response"),
        }
    }

    pub async fn progress(&self, status: &str, detail: &str) -> anyhow::Result<()> {
        self.send(&DroneMessage::Progress(Progress {
            status: status.to_string(),
            detail: Some(detail.to_string()),
        }))
        .await
    }
}

pub async fn run(runner: impl DroneRunner) -> anyhow::Result<()> {
    let channel = QueenChannel::new();

    let msg = channel.recv().await?;
    let job = match msg {
        QueenMessage::Job(spec) => spec,
        _ => anyhow::bail!("expected Job message from queen, got: {:?}", msg),
    };

    tracing::info!(job_run_id = %job.job_run_id, "drone starting");

    let env = match runner.setup(&job).await {
        Ok(env) => env,
        Err(e) => {
            channel
                .send(&DroneMessage::Error(DroneError {
                    message: format!("setup failed: {e}"),
                }))
                .await?;
            return Err(e);
        }
    };

    channel
        .progress("setup_complete", "environment ready")
        .await?;

    let result = match runner.execute(&env, &channel).await {
        Ok(output) => output,
        Err(e) => {
            channel
                .send(&DroneMessage::Error(DroneError {
                    message: format!("execution failed: {e}"),
                }))
                .await?;
            runner.teardown(&env).await;
            return Err(e);
        }
    };

    channel.send(&DroneMessage::Result(result)).await?;
    runner.teardown(&env).await;

    tracing::info!(job_run_id = %job.job_run_id, "drone finished");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queen_channel_creates() {
        let _ = QueenChannel::new();
    }
}
