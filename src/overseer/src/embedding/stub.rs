use std::future::Future;
use std::pin::Pin;

use super::EmbeddingProvider;
use crate::error::Result;

pub struct StubEmbedding {
    dims: usize,
}

impl StubEmbedding {
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }
}

impl EmbeddingProvider for StubEmbedding {
    fn embed(&self, _text: &str) -> Pin<Box<dyn Future<Output = Result<Vec<f32>>> + Send + '_>> {
        let dims = self.dims;
        Box::pin(async move { Ok(vec![0.0; dims]) })
    }

    fn model_name(&self) -> &str {
        "stub"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_returns_zero_vector() {
        let stub = StubEmbedding::new(384);
        let embedding = stub.embed("anything").await.unwrap();
        assert_eq!(embedding.len(), 384);
        assert!(embedding.iter().all(|&v| v == 0.0));
    }

    #[tokio::test]
    async fn test_stub_model_name() {
        let stub = StubEmbedding::new(384);
        assert_eq!(stub.model_name(), "stub");
        assert_eq!(stub.dimensions(), 384);
    }
}
