use std::path::Path;

use crate::config::AbathurConfig;
use crate::index::Index;
use crate::staleness;

pub struct Generator {
    config: AbathurConfig,
}

impl Generator {
    pub fn new(config: AbathurConfig) -> Self {
        Self { config }
    }

    /// Generate an abathur doc for the given source path by calling the Claude API.
    /// Reads the source files, sends them as context with the schema prompt,
    /// and returns the generated markdown document.
    pub async fn generate(&self, source_path: &Path) -> anyhow::Result<String> {
        let source_content = std::fs::read_to_string(source_path)?;
        let schema = crate::code::code_prompt();

        let api_key = std::env::var(&self.config.generate.api_key_env).map_err(|_| {
            anyhow::anyhow!(
                "environment variable '{}' not set",
                self.config.generate.api_key_env
            )
        })?;

        let prompt = format!(
            "{schema}\n\n---\n\n\
             Generate an abathur document for the following source file.\n\
             Source path: {path}\n\n\
             ```rust\n{source_content}\n```",
            schema = schema,
            path = source_path.display(),
            source_content = source_content,
        );

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": self.config.generate.model,
                "max_tokens": 4096,
                "messages": [{
                    "role": "user",
                    "content": prompt,
                }]
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Claude API error {status}: {body}");
        }

        let body: serde_json::Value = response.json().await?;
        let text = body["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("unexpected API response format"))?;

        Ok(text.to_string())
    }

    /// Regenerate all stale docs.
    pub async fn regenerate_stale(&self, index: &Index) -> anyhow::Result<Vec<String>> {
        let stale = staleness::check(index)?;
        let mut updated = Vec::new();

        for stale_doc in &stale {
            let meta = index
                .docs
                .get(&stale_doc.slug)
                .ok_or_else(|| anyhow::anyhow!("stale doc '{}' not in index", stale_doc.slug))?;

            // Regenerate from first source file
            if let Some(first_source) = meta.sources.first() {
                let content = self.generate(&first_source.path).await?;
                std::fs::write(&meta.path, &content)?;
                crate::hash::update_hashes(&meta.path)?;
                updated.push(stale_doc.slug.clone());
            }
        }

        Ok(updated)
    }
}
