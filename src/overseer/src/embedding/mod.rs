pub mod stub;
pub mod voyage;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::Result;

pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, text: &str) -> Pin<Box<dyn Future<Output = Result<Vec<f32>>> + Send + '_>>;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}

pub struct EmbeddingRegistry {
    providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
    default: String,
}

impl EmbeddingRegistry {
    pub fn new(
        providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
        default: String,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            providers.contains_key(&default),
            "default provider '{default}' not found in registry"
        );
        Ok(Self { providers, default })
    }

    pub fn get_default(&self) -> &Arc<dyn EmbeddingProvider> {
        &self.providers[&self.default]
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn EmbeddingProvider>> {
        self.providers.get(name)
    }

    pub fn default_name(&self) -> &str {
        &self.default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stub::StubEmbedding;

    fn make_registry() -> EmbeddingRegistry {
        let mut providers: HashMap<String, Arc<dyn EmbeddingProvider>> = HashMap::new();
        providers.insert("stub".into(), Arc::new(StubEmbedding::new(384)));
        providers.insert("other".into(), Arc::new(StubEmbedding::new(768)));
        EmbeddingRegistry::new(providers, "stub".into()).unwrap()
    }

    #[test]
    fn test_registry_get_default() {
        let reg = make_registry();
        assert_eq!(reg.default_name(), "stub");
        assert_eq!(reg.get_default().dimensions(), 384);
    }

    #[test]
    fn test_registry_get_by_name() {
        let reg = make_registry();
        assert!(reg.get("stub").is_some());
        assert!(reg.get("other").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_invalid_default() {
        let providers: HashMap<String, Arc<dyn EmbeddingProvider>> = HashMap::new();
        let result = EmbeddingRegistry::new(providers, "missing".into());
        assert!(result.is_err());
    }
}
