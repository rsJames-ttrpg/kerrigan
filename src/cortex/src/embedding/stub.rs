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
    fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; self.dims])
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

    #[test]
    fn test_stub_returns_zero_vector() {
        let stub = StubEmbedding::new(384);
        let embedding = stub.embed("anything").unwrap();
        assert_eq!(embedding.len(), 384);
        assert!(embedding.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_stub_model_name() {
        let stub = StubEmbedding::new(384);
        assert_eq!(stub.model_name(), "stub");
        assert_eq!(stub.dimensions(), 384);
    }
}
