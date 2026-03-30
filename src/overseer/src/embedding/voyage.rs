use std::future::Future;
use std::pin::Pin;

use super::EmbeddingProvider;
use crate::error::{OverseerError, Result};

pub struct VoyageEmbedding {
    model: String,
    api_key: String,
    dimensions: usize,
}

impl VoyageEmbedding {
    pub fn new(model: String, api_key: String, dimensions: usize) -> Self {
        Self {
            model,
            api_key,
            dimensions,
        }
    }
}

impl EmbeddingProvider for VoyageEmbedding {
    fn embed(&self, text: &str) -> Pin<Box<dyn Future<Output = Result<Vec<f32>>> + Send + '_>> {
        let text = text.to_string();
        Box::pin(async move {
            let model = self.model.clone();
            let api_key = self.api_key.clone();
            let dimensions = self.dimensions;

            let result = tokio::task::spawn_blocking(move || {
                let body = serde_json::json!({
                    "input": [&text],
                    "model": &model,
                });

                let mut response = ureq::post("https://api.voyageai.com/v1/embeddings")
                    .header("Authorization", &format!("Bearer {api_key}"))
                    .send_json(&body)
                    .map_err(|e| OverseerError::Embedding(e.to_string()))?;

                let json: serde_json::Value = response
                    .body_mut()
                    .read_json()
                    .map_err(|e| OverseerError::Embedding(e.to_string()))?;

                let embedding: Vec<f32> = json["data"][0]["embedding"]
                    .as_array()
                    .ok_or_else(|| OverseerError::Embedding("unexpected response shape".into()))?
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect();

                if embedding.len() != dimensions {
                    return Err(OverseerError::Embedding(format!(
                        "dimension mismatch: expected {dimensions}, got {}",
                        embedding.len()
                    )));
                }

                Ok(embedding)
            })
            .await
            .map_err(|e| OverseerError::Embedding(e.to_string()))??;

            Ok(result)
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}
