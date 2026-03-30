pub mod stub;

use crate::error::Result;

pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}
