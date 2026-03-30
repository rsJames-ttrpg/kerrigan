use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::{OverseerError, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ArtifactMetadata {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub run_id: Option<String>,
    pub created_at: String,
}

fn row_to_artifact(row: &sqlx::sqlite::SqliteRow) -> ArtifactMetadata {
    ArtifactMetadata {
        id: row.get("id"),
        name: row.get("name"),
        content_type: row.get("content_type"),
        size: row.get("size"),
        run_id: row.get("run_id"),
        created_at: row.get("created_at"),
    }
}

pub async fn insert_artifact(
    pool: &SqlitePool,
    name: &str,
    content_type: &str,
    size: i64,
    run_id: Option<&str>,
) -> Result<ArtifactMetadata> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO artifacts (id, name, content_type, size, run_id) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(&id)
    .bind(name)
    .bind(content_type)
    .bind(size)
    .bind(run_id)
    .execute(pool)
    .await
    .map_err(OverseerError::Storage)?;

    get_artifact(pool, &id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("artifact {id}")))
}

pub async fn get_artifact(pool: &SqlitePool, id: &str) -> Result<Option<ArtifactMetadata>> {
    let row = sqlx::query(
        "SELECT id, name, content_type, size, run_id, created_at \
         FROM artifacts WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_artifact))
}

pub async fn list_artifacts(
    pool: &SqlitePool,
    run_id: Option<&str>,
) -> Result<Vec<ArtifactMetadata>> {
    let rows = sqlx::query(
        "SELECT id, name, content_type, size, run_id, created_at \
         FROM artifacts \
         WHERE (?1 IS NULL OR run_id = ?1) \
         ORDER BY created_at",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(row_to_artifact).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{jobs as db_jobs, open_in_memory_named};

    #[tokio::test]
    async fn test_artifact_crud() {
        let pool = open_in_memory_named("artifacts_test_crud")
            .await
            .expect("pool opens");

        let artifact = insert_artifact(&pool, "report.pdf", "application/pdf", 1024, None)
            .await
            .expect("insert succeeds");

        assert!(!artifact.id.is_empty());
        assert_eq!(artifact.name, "report.pdf");
        assert_eq!(artifact.content_type, "application/pdf");
        assert_eq!(artifact.size, 1024);
        assert!(artifact.run_id.is_none());

        let fetched = get_artifact(&pool, &artifact.id)
            .await
            .expect("get succeeds")
            .expect("artifact exists");
        assert_eq!(fetched.id, artifact.id);
        assert_eq!(fetched.name, artifact.name);
        assert_eq!(fetched.size, artifact.size);

        let all = list_artifacts(&pool, None).await.expect("list all");
        assert_eq!(all.len(), 1);

        let by_run = list_artifacts(&pool, Some("nonexistent-run"))
            .await
            .expect("list by run");
        assert!(by_run.is_empty());
    }

    #[tokio::test]
    async fn test_artifact_list_by_run() {
        let pool = open_in_memory_named("artifacts_test_list_by_run")
            .await
            .expect("pool opens");

        // Create real job definition and run to satisfy FK constraints
        let def = db_jobs::create_job_definition(
            &pool,
            "artifact-test-job",
            "desc",
            serde_json::json!({}),
        )
        .await
        .expect("create def");
        let run = db_jobs::start_job_run(&pool, &def.id, "tester", None)
            .await
            .expect("start run");

        insert_artifact(&pool, "file1.txt", "text/plain", 100, Some(&run.id))
            .await
            .expect("insert 1");
        insert_artifact(&pool, "file2.txt", "text/plain", 200, Some(&run.id))
            .await
            .expect("insert 2");
        insert_artifact(&pool, "file3.txt", "text/plain", 300, None)
            .await
            .expect("insert 3");

        let by_run = list_artifacts(&pool, Some(&run.id))
            .await
            .expect("list by run");
        assert_eq!(by_run.len(), 2);

        let all = list_artifacts(&pool, None).await.expect("list all");
        assert_eq!(all.len(), 3);
    }
}
