pub mod report;

mod fetch;
mod metrics;
mod parse;
mod rules;

use chrono::{DateTime, Utc};
use nydus::NydusClient;

use report::{AnalysisReport, AnalysisScope};

pub struct EvolutionChamber {
    client: NydusClient,
}

impl EvolutionChamber {
    pub fn new(client: NydusClient) -> Self {
        Self { client }
    }

    pub async fn analyze(
        &self,
        scope: AnalysisScope,
        since: DateTime<Utc>,
        min_sessions: usize,
    ) -> anyhow::Result<Option<AnalysisReport>> {
        todo!()
    }
}
