use std::sync::Arc;

use object_store::ObjectStore;
use object_store::ObjectStoreExt;
use object_store::PutPayload;
use object_store::path::Path as ObjectPath;

use crate::db::Database;
use crate::db::models::ArtifactMetadata;
use crate::error::{OverseerError, Result};

pub struct ArtifactService {
    db: Arc<dyn Database>,
    store: Arc<dyn ObjectStore>,
}

impl ArtifactService {
    pub fn new(db: Arc<dyn Database>, store: Arc<dyn ObjectStore>) -> Self {
        Self { db, store }
    }

    pub async fn store(
        &self,
        name: &str,
        content_type: &str,
        data: &[u8],
        run_id: Option<&str>,
    ) -> Result<ArtifactMetadata> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = ObjectPath::from(format!("artifacts/{id}"));
        self.store
            .put(&path, PutPayload::from(data.to_vec()))
            .await?;
        self.db
            .insert_artifact(&id, name, content_type, data.len() as i64, run_id)
            .await
    }

    pub async fn get(&self, id: &str) -> Result<(ArtifactMetadata, Vec<u8>)> {
        let metadata = self
            .db
            .get_artifact(id)
            .await?
            .ok_or_else(|| OverseerError::NotFound(format!("artifact {id}")))?;
        let path = ObjectPath::from(format!("artifacts/{id}"));
        let result: object_store::GetResult = self.store.get(&path).await?;
        let data = result
            .bytes()
            .await
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;
        Ok((metadata, data.to_vec()))
    }

    pub async fn list(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>> {
        self.db.list_artifacts(run_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDatabase;
    use crate::storage::create_in_memory_store;

    #[tokio::test]
    async fn test_artifact_service_store_and_get() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_artifacts_test_store")
            .await
            .expect("db opens");
        let store = create_in_memory_store();
        let svc = ArtifactService::new(Arc::new(sqlite_db), store);

        let data = b"hello artifact world";
        let meta = svc
            .store("hello.txt", "text/plain", data, None)
            .await
            .expect("store succeeds");

        assert_eq!(meta.name, "hello.txt");
        assert_eq!(meta.content_type, "text/plain");
        assert_eq!(meta.size, data.len() as i64);

        let (fetched_meta, fetched_data) = svc.get(&meta.id).await.expect("get succeeds");
        assert_eq!(fetched_meta.id, meta.id);
        assert_eq!(fetched_data, data);
    }

    #[tokio::test]
    async fn test_artifact_service_list() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_artifacts_test_list")
            .await
            .expect("db opens");
        let store = create_in_memory_store();
        let svc = ArtifactService::new(Arc::new(sqlite_db), store);

        svc.store("a.bin", "application/octet-stream", b"aaa", None)
            .await
            .expect("store a");
        svc.store("b.bin", "application/octet-stream", b"bbb", None)
            .await
            .expect("store b");

        let all = svc.list(None).await.expect("list all");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_artifact_service_get_not_found() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_artifacts_test_notfound")
            .await
            .expect("db opens");
        let store = create_in_memory_store();
        let svc = ArtifactService::new(Arc::new(sqlite_db), store);

        let result = svc.get("nonexistent-id").await;
        assert!(
            matches!(result, Err(OverseerError::NotFound(_))),
            "expected NotFound, got: {result:?}"
        );
    }
}
