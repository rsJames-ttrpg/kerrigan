use sqlx::SqlitePool;
use std::sync::Arc;

use crate::db::memory as db;
use crate::embedding::EmbeddingProvider;
use crate::error::Result;

pub struct MemoryService {
    pool: SqlitePool,
    embedder: Arc<dyn EmbeddingProvider>,
}

impl MemoryService {
    pub fn new(pool: SqlitePool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self { pool, embedder }
    }

    pub async fn store(
        &self,
        content: &str,
        source: &str,
        tags: &[String],
        expires_at: Option<&str>,
    ) -> Result<db::Memory> {
        let embedding = self.embedder.embed(content)?;
        let model = self.embedder.model_name().to_string();
        db::insert_memory(
            &self.pool, content, &embedding, &model, source, tags, expires_at,
        )
        .await
    }

    pub async fn recall(
        &self,
        query: &str,
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<db::MemorySearchResult>> {
        let embedding = self.embedder.embed(query)?;
        db::search_memories(&self.pool, &embedding, tags_filter, limit).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        db::delete_memory(&self.pool, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::embedding::stub::StubEmbedding;

    async fn make_service() -> MemoryService {
        let pool = open_in_memory().await.expect("pool opens");
        let embedder = Arc::new(StubEmbedding::new(384));
        MemoryService::new(pool, embedder)
    }

    #[tokio::test]
    async fn test_memory_service_store_and_recall() {
        let svc = make_service().await;

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
        let svc = make_service().await;

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
