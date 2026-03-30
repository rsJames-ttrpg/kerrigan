use chrono::NaiveDateTime;
use sea_query::{Expr, Order, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};

pub use super::models::ArtifactMetadata;
use super::tables::Artifacts;
use crate::error::{OverseerError, Result};

fn row_to_artifact(row: &sqlx::sqlite::SqliteRow) -> ArtifactMetadata {
    ArtifactMetadata {
        id: row.get("id"),
        name: row.get("name"),
        content_type: row.get("content_type"),
        size: row.get("size"),
        run_id: row.get("run_id"),
        created_at: row.get::<NaiveDateTime, _>("created_at").and_utc(),
    }
}

pub async fn insert_artifact(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    content_type: &str,
    size: i64,
    run_id: Option<&str>,
) -> Result<ArtifactMetadata> {
    let (sql, values) = Query::insert()
        .into_table(Artifacts::Table)
        .columns([
            Artifacts::Id,
            Artifacts::Name,
            Artifacts::ContentType,
            Artifacts::Size,
            Artifacts::RunId,
        ])
        .values_panic([
            id.into(),
            name.into(),
            content_type.into(),
            size.into(),
            run_id.map(|s| s.to_string()).into(),
        ])
        .returning(Query::returning().columns([
            Artifacts::Id,
            Artifacts::Name,
            Artifacts::ContentType,
            Artifacts::Size,
            Artifacts::RunId,
            Artifacts::CreatedAt,
        ]))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_artifact(&row))
}

pub async fn get_artifact(pool: &SqlitePool, id: &str) -> Result<Option<ArtifactMetadata>> {
    let (sql, values) = Query::select()
        .columns([
            Artifacts::Id,
            Artifacts::Name,
            Artifacts::ContentType,
            Artifacts::Size,
            Artifacts::RunId,
            Artifacts::CreatedAt,
        ])
        .from(Artifacts::Table)
        .and_where(Expr::col(Artifacts::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_artifact))
}

pub async fn list_artifacts(
    pool: &SqlitePool,
    run_id: Option<&str>,
) -> Result<Vec<ArtifactMetadata>> {
    let mut query = Query::select();
    query
        .columns([
            Artifacts::Id,
            Artifacts::Name,
            Artifacts::ContentType,
            Artifacts::Size,
            Artifacts::RunId,
            Artifacts::CreatedAt,
        ])
        .from(Artifacts::Table);

    if let Some(rid) = run_id {
        query.and_where(Expr::col(Artifacts::RunId).eq(rid));
    }

    query.order_by(Artifacts::CreatedAt, Order::Asc);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
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

        let artifact = insert_artifact(
            &pool,
            "test-id-1",
            "report.pdf",
            "application/pdf",
            1024,
            None,
        )
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

        insert_artifact(&pool, "id-1", "file1.txt", "text/plain", 100, Some(&run.id))
            .await
            .expect("insert 1");
        insert_artifact(&pool, "id-2", "file2.txt", "text/plain", 200, Some(&run.id))
            .await
            .expect("insert 2");
        insert_artifact(&pool, "id-3", "file3.txt", "text/plain", 300, None)
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
