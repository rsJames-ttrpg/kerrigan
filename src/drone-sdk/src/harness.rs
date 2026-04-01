use std::io::Write;

use crate::protocol::*;
use crate::runner::DroneRunner;

pub struct QueenChannel {
    writer: std::io::Stdout,
    reader: std::io::BufReader<std::io::Stdin>,
}

impl QueenChannel {
    fn new() -> Self {
        Self {
            writer: std::io::stdout(),
            reader: std::io::BufReader::new(std::io::stdin()),
        }
    }

    fn send(&mut self, msg: &DroneMessage) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    fn recv(&mut self) -> anyhow::Result<QueenMessage> {
        use std::io::BufRead;
        let mut line = String::new();
        let bytes_read = self.reader.read_line(&mut line)?;
        if bytes_read == 0 {
            anyhow::bail!("queen disconnected (stdin closed)");
        }
        let msg: QueenMessage = serde_json::from_str(line.trim()).map_err(|e| {
            anyhow::anyhow!("failed to parse queen message: {e}, raw: {:?}", line.trim())
        })?;
        Ok(msg)
    }

    pub fn request_auth(&mut self, url: &str, message: &str) -> anyhow::Result<AuthResponse> {
        self.send(&DroneMessage::AuthRequest(AuthRequest {
            url: url.to_string(),
            message: message.to_string(),
        }))?;
        let msg = self.recv()?;
        match msg {
            QueenMessage::AuthResponse(resp) => Ok(resp),
            QueenMessage::Cancel {} => anyhow::bail!("cancelled by queen"),
            _ => anyhow::bail!("unexpected message from queen: expected auth_response"),
        }
    }

    pub fn progress(&mut self, status: &str, detail: &str) -> anyhow::Result<()> {
        self.send(&DroneMessage::Progress(Progress {
            status: status.to_string(),
            detail: Some(detail.to_string()),
        }))
    }
}

pub async fn run(runner: impl DroneRunner) -> anyhow::Result<()> {
    let mut channel = QueenChannel::new();

    let msg = channel.recv()?;
    let job = match msg {
        QueenMessage::Job(spec) => spec,
        _ => anyhow::bail!("expected Job message from queen, got: {:?}", msg),
    };

    tracing::info!(job_run_id = %job.job_run_id, "drone starting");

    let env = match runner.setup(&job).await {
        Ok(env) => env,
        Err(e) => {
            channel.send(&DroneMessage::Error(DroneError {
                message: format!("setup failed: {e}"),
            }))?;
            return Err(e);
        }
    };

    channel.progress("setup_complete", "environment ready")?;

    let result = match runner.execute(&env, &mut channel).await {
        Ok(output) => output,
        Err(e) => {
            channel.send(&DroneMessage::Error(DroneError {
                message: format!("execution failed: {e}"),
            }))?;
            runner.teardown(&env).await;
            return Err(e);
        }
    };

    channel.send(&DroneMessage::Result(result))?;
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
