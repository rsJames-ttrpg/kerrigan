use sqlx::SqlitePool;

use crate::db::memory as db;
use crate::embedding::EmbeddingRegistry;
use crate::error::Result;

pub struct MemoryService {
    pool: SqlitePool,
    registry: EmbeddingRegistry,
}

impl MemoryService {
    pub fn new(pool: SqlitePool, registry: EmbeddingRegistry) -> Self {
        Self { pool, registry }
    }

    pub async fn store(
        &self,
        content: &str,
        source: &str,
        tags: &[String],
        expires_at: Option<&str>,
    ) -> Result<db::Memory> {
        let provider = self.registry.get_default();
        let provider_name = self.registry.default_name();
        let embedding = provider.embed(content).await?;
        db::insert_memory(
            &self.pool,
            provider_name,
            content,
            &embedding,
            provider_name,
            source,
            tags,
            expires_at,
        )
        .await
    }

    pub async fn recall(
        &self,
        query: &str,
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<db::MemorySearchResult>> {
        let provider = self.registry.get_default();
        let provider_name = self.registry.default_name();
        let embedding = provider.embed(query).await?;
        db::search_memories(&self.pool, provider_name, &embedding, tags_filter, limit).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let memory = db::get_memory(&self.pool, id).await?;
        db::delete_memory(&self.pool, &memory.embedding_model, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_embedding_table, open_in_memory_named};
    use crate::embedding::EmbeddingProvider;
    use crate::embedding::stub::StubEmbedding;
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn make_service(name: &str) -> MemoryService {
        let pool = open_in_memory_named(name).await.expect("pool opens");
        create_embedding_table(&pool, "stub", 384)
            .await
            .expect("create table");
        let mut providers: HashMap<String, Arc<dyn EmbeddingProvider>> = HashMap::new();
        providers.insert("stub".into(), Arc::new(StubEmbedding::new(384)));
        let registry = EmbeddingRegistry::new(providers, "stub".into()).unwrap();
        MemoryService::new(pool, registry)
    }

    #[tokio::test]
    async fn test_memory_service_store_and_recall() {
        let svc = make_service("svc_test_store_recall").await;

        let tags = vec!["svc-test".to_string()];
        let memory = svc
            .store("service memory content", "service-test", &tags, None)
            .await
            .expect("store succeeds");

        assert_eq!(memory.content, "service memory content");
        assert_eq!(memory.source, "service-test");
        assert_eq!(memory.tags, tags);

        let results = svc
            .recall("service memory content", None, 10)
            .await
            .expect("recall succeeds");

        assert!(
            results.iter().any(|r| r.memory.id == memory.id),
            "stored memory should appear in recall results"
        );
    }

    #[tokio::test]
    async fn test_memory_service_delete() {
        let svc = make_service("svc_test_delete").await;

        let memory = svc
            .store("to be deleted via service", "service-test", &[], None)
            .await
            .expect("store succeeds");

        svc.delete(&memory.id).await.expect("delete succeeds");

        // Recall should return empty since the only memory was deleted
        let results = svc
            .recall("to be deleted via service", None, 10)
            .await
            .expect("recall succeeds");

        assert!(
            results.iter().all(|r| r.memory.id != memory.id),
            "deleted memory should not appear in results"
        );
    }
}
