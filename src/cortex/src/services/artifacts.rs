use sqlx::SqlitePool;
use std::path::PathBuf;
use tokio::fs;

use crate::db::artifacts as db;
use crate::error::{CortexError, Result};

pub struct ArtifactService {
    pool: SqlitePool,
    artifact_path: PathBuf,
}

impl ArtifactService {
    pub fn new(pool: SqlitePool, artifact_path: PathBuf) -> Self {
        Self {
            pool,
            artifact_path,
        }
    }

    pub async fn store(
        &self,
        name: &str,
        content_type: &str,
        data: &[u8],
        run_id: Option<&str>,
    ) -> Result<db::ArtifactMetadata> {
        // Insert metadata first to get an ID
        let metadata =
            db::insert_artifact(&self.pool, name, content_type, data.len() as i64, run_id).await?;

        // Write blob to filesystem at <artifact_path>/<id>
        let dest = self.artifact_path.join(&metadata.id);
        fs::create_dir_all(&self.artifact_path).await?;
        fs::write(&dest, data).await?;

        Ok(metadata)
    }

    pub async fn get(&self, id: &str) -> Result<(db::ArtifactMetadata, Vec<u8>)> {
        let metadata = db::get_artifact(&self.pool, id)
            .await?
            .ok_or_else(|| CortexError::NotFound(format!("artifact {id}")))?;

        let path = self.artifact_path.join(id);
        let data = fs::read(&path).await?;

        Ok((metadata, data))
    }

    pub async fn list(&self, run_id: Option<&str>) -> Result<Vec<db::ArtifactMetadata>> {
        db::list_artifacts(&self.pool, run_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_named;

    fn test_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cortex-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_artifact_service_store_and_get() {
        let pool = open_in_memory_named("svc_artifacts_test_store")
            .await
            .expect("pool opens");
        let dir = test_dir();
        let svc = ArtifactService::new(pool, dir);

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
        let pool = open_in_memory_named("svc_artifacts_test_list")
            .await
            .expect("pool opens");
        let dir = test_dir();
        let svc = ArtifactService::new(pool, dir);

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
        let pool = open_in_memory_named("svc_artifacts_test_notfound")
            .await
            .expect("pool opens");
        let dir = test_dir();
        let svc = ArtifactService::new(pool, dir);

        let result = svc.get("nonexistent-id").await;
        assert!(
            matches!(result, Err(CortexError::NotFound(_))),
            "expected NotFound, got: {result:?}"
        );
    }
}
