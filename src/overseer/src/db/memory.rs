use sea_query::{Expr, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;
use zerocopy::IntoBytes;

pub use super::models::{Memory, MemorySearchResult};
use super::tables::{Memories, MemoryLinks};
use crate::error::{OverseerError, Result};

fn row_to_memory(row: &sqlx::sqlite::SqliteRow) -> Memory {
    let tags_json: String = row.get("tags");
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    Memory {
        id: row.get("id"),
        content: row.get("content"),
        embedding_model: row.get("embedding_model"),
        source: row.get("source"),
        tags,
        expires_at: row.get("expires_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_memory(
    pool: &SqlitePool,
    provider_name: &str,
    content: &str,
    embedding: &[f32],
    embedding_model: &str,
    source: &str,
    tags: &[String],
    expires_at: Option<&str>,
) -> Result<Memory> {
    let id = Uuid::new_v4().to_string();
    let tags_json =
        serde_json::to_string(tags).map_err(|e| OverseerError::Internal(e.to_string()))?;

    let mut tx = pool.begin().await.map_err(OverseerError::Storage)?;

    // Insert into memories table and get the rowid — use sea-query for the metadata INSERT
    let (sql, values) = Query::insert()
        .into_table(Memories::Table)
        .columns([
            Memories::Id,
            Memories::Content,
            Memories::EmbeddingModel,
            Memories::Source,
            Memories::Tags,
            Memories::ExpiresAt,
        ])
        .values_panic([
            id.clone().into(),
            content.into(),
            embedding_model.into(),
            source.into(),
            tags_json.into(),
            expires_at.map(|s| s.to_string()).into(),
        ])
        .returning_col(sea_query::Alias::new("rowid"))
        .build_sqlx(SqliteQueryBuilder);

    let rowid: i64 = sqlx::query_scalar_with(&sql, values)
        .fetch_one(&mut *tx)
        .await
        .map_err(OverseerError::Storage)?;

    // Insert into memory_embeddings virtual table using the same rowid — raw SQL (sqlite-vec)
    let embedding_bytes: &[u8] = embedding.as_bytes();
    let emb_sql =
        format!("INSERT INTO memory_embeddings_{provider_name} (rowid, embedding) VALUES (?1, ?2)");
    sqlx::query(&emb_sql)
        .bind(rowid)
        .bind(embedding_bytes)
        .execute(&mut *tx)
        .await
        .map_err(OverseerError::Storage)?;

    tx.commit().await.map_err(OverseerError::Storage)?;

    get_memory(pool, &id).await
}

pub async fn get_memory(pool: &SqlitePool, id: &str) -> Result<Memory> {
    let (sql, values) = Query::select()
        .columns([
            Memories::Id,
            Memories::Content,
            Memories::EmbeddingModel,
            Memories::Source,
            Memories::Tags,
            Memories::ExpiresAt,
            Memories::CreatedAt,
            Memories::UpdatedAt,
        ])
        .from(Memories::Table)
        .and_where(Expr::col(Memories::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?
        .ok_or_else(|| OverseerError::NotFound(format!("memory {id}")))?;

    Ok(row_to_memory(&row))
}

pub async fn delete_memory(pool: &SqlitePool, provider_name: &str, id: &str) -> Result<()> {
    // Get the rowid first so we can clean up the embedding
    let rowid: Option<i64> = sqlx::query_scalar("SELECT rowid FROM memories WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    // Delete from memories table using sea-query
    let (sql, values) = Query::delete()
        .from_table(Memories::Table)
        .and_where(Expr::col(Memories::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    // Clean up the vec0 embedding row to avoid KNN scan bloat — raw SQL (sqlite-vec)
    if let Some(rowid) = rowid {
        let del_sql = format!("DELETE FROM memory_embeddings_{provider_name} WHERE rowid = ?1");
        let _ = sqlx::query(&del_sql).bind(rowid).execute(pool).await;
    }

    Ok(())
}

pub async fn search_memories(
    pool: &SqlitePool,
    provider_name: &str,
    query_embedding: &[f32],
    tags_filter: Option<&[String]>,
    limit: usize,
) -> Result<Vec<MemorySearchResult>> {
    let embedding_bytes: &[u8] = query_embedding.as_bytes();
    // Fetch more than `limit` so we have room to post-filter by tags
    let fetch_limit = (limit * 10).max(100) as i64;

    let sql = format!(
        "SELECT m.id, m.content, m.embedding_model, m.source, m.tags, m.expires_at, \
         m.created_at, m.updated_at, v.distance \
         FROM memory_embeddings_{provider_name} v \
         JOIN memories m ON m.rowid = v.rowid \
         WHERE v.embedding MATCH ?1 AND k = ?2 \
         ORDER BY v.distance"
    );
    let rows = sqlx::query(&sql)
        .bind(embedding_bytes)
        .bind(fetch_limit)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    let mut results: Vec<MemorySearchResult> = rows
        .iter()
        .map(|row| {
            let distance: f64 = row.get("distance");
            MemorySearchResult {
                memory: row_to_memory(row),
                distance,
            }
        })
        .collect();

    if let Some(filter_tags) = tags_filter
        && !filter_tags.is_empty()
    {
        results.retain(|r| filter_tags.iter().any(|ft| r.memory.tags.contains(ft)));
    }

    results.truncate(limit);
    Ok(results)
}

pub async fn insert_memory_link(
    pool: &SqlitePool,
    memory_id: &str,
    linked_id: &str,
    linked_type: &str,
    relation_type: &str,
) -> Result<()> {
    let (sql, values) = Query::insert()
        .into_table(MemoryLinks::Table)
        .columns([
            MemoryLinks::MemoryId,
            MemoryLinks::LinkedId,
            MemoryLinks::LinkedType,
            MemoryLinks::RelationType,
        ])
        .values_panic([
            memory_id.into(),
            linked_id.into(),
            linked_type.into(),
            relation_type.into(),
        ])
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_embedding_table, open_in_memory_named};

    fn make_embedding(val: f32) -> Vec<f32> {
        vec![val; 384]
    }

    #[tokio::test]
    async fn test_insert_and_get_memory() {
        let pool = open_in_memory_named("mem_test_insert_get")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "stub", 384)
            .await
            .expect("create table");
        let tags = vec!["test".to_string(), "demo".to_string()];
        let embedding = make_embedding(0.1);

        let memory = insert_memory(
            &pool,
            "stub",
            "hello world",
            &embedding,
            "stub",
            "unit-test",
            &tags,
            None,
        )
        .await
        .expect("insert succeeds");

        assert!(!memory.id.is_empty());
        assert_eq!(memory.content, "hello world");
        assert_eq!(memory.embedding_model, "stub");
        assert_eq!(memory.source, "unit-test");
        assert_eq!(memory.tags, tags);
        assert!(memory.expires_at.is_none());

        // Fetch back
        let fetched = get_memory(&pool, &memory.id).await.expect("get succeeds");
        assert_eq!(fetched.id, memory.id);
        assert_eq!(fetched.content, memory.content);
        assert_eq!(fetched.tags, memory.tags);
    }

    #[tokio::test]
    async fn test_delete_memory() {
        let pool = open_in_memory_named("mem_test_delete")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "stub", 384)
            .await
            .expect("create table");
        let embedding = make_embedding(0.0);

        let memory = insert_memory(
            &pool,
            "stub",
            "to be deleted",
            &embedding,
            "stub",
            "unit-test",
            &[],
            None,
        )
        .await
        .expect("insert succeeds");

        delete_memory(&pool, "stub", &memory.id)
            .await
            .expect("delete succeeds");

        let result = get_memory(&pool, &memory.id).await;
        assert!(
            matches!(result, Err(OverseerError::NotFound(_))),
            "expected NotFound, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_search_memories() {
        let pool = open_in_memory_named("mem_test_search")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "stub", 384)
            .await
            .expect("create table");

        // Insert 3 memories with distinct embeddings
        let emb_a = make_embedding(1.0);
        let emb_b = make_embedding(0.5);
        let emb_c = make_embedding(0.1);

        let mem_a = insert_memory(
            &pool,
            "stub",
            "close one",
            &emb_a,
            "stub",
            "test",
            &[],
            None,
        )
        .await
        .expect("insert a");
        let _mem_b = insert_memory(
            &pool,
            "stub",
            "middle one",
            &emb_b,
            "stub",
            "test",
            &[],
            None,
        )
        .await
        .expect("insert b");
        let _mem_c = insert_memory(&pool, "stub", "far one", &emb_c, "stub", "test", &[], None)
            .await
            .expect("insert c");

        // Query with embedding closest to emb_a
        let query = make_embedding(1.0);
        let results = search_memories(&pool, "stub", &query, None, 3)
            .await
            .expect("search succeeds");

        assert_eq!(results.len(), 3);
        // The closest should be mem_a
        assert_eq!(results[0].memory.id, mem_a.id);
        // Results should be ordered by increasing distance
        assert!(results[0].distance <= results[1].distance);
        assert!(results[1].distance <= results[2].distance);
    }

    #[tokio::test]
    async fn test_delete_memory_cleans_up_embeddings() {
        let pool = open_in_memory_named("mem_test_delete_cleanup")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "stub", 384)
            .await
            .expect("create table");
        let embedding = make_embedding(0.5);

        let memory = insert_memory(
            &pool,
            "stub",
            "will delete",
            &embedding,
            "stub",
            "test",
            &[],
            None,
        )
        .await
        .expect("insert succeeds");

        // Get the rowid for this memory
        let rowid: i64 = sqlx::query_scalar("SELECT rowid FROM memories WHERE id = ?1")
            .bind(&memory.id)
            .fetch_one(&pool)
            .await
            .expect("rowid exists");

        // Verify embedding exists before delete
        let count_before: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM memory_embeddings_stub WHERE rowid = ?1")
                .bind(rowid)
                .fetch_one(&pool)
                .await
                .expect("count query");
        assert_eq!(count_before, 1);

        delete_memory(&pool, "stub", &memory.id)
            .await
            .expect("delete succeeds");

        // Verify embedding is gone after delete
        let count_after: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM memory_embeddings_stub WHERE rowid = ?1")
                .bind(rowid)
                .fetch_one(&pool)
                .await
                .expect("count query");
        assert_eq!(count_after, 0);
    }

    #[tokio::test]
    async fn test_search_memories_with_tag_filter() {
        let pool = open_in_memory_named("mem_test_search_tags")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "stub", 384)
            .await
            .expect("create table");

        let emb = make_embedding(1.0);
        insert_memory(
            &pool,
            "stub",
            "tagged a",
            &emb,
            "stub",
            "test",
            &["alpha".into()],
            None,
        )
        .await
        .expect("insert a");
        insert_memory(
            &pool,
            "stub",
            "tagged b",
            &emb,
            "stub",
            "test",
            &["beta".into()],
            None,
        )
        .await
        .expect("insert b");
        insert_memory(
            &pool,
            "stub",
            "tagged both",
            &emb,
            "stub",
            "test",
            &["alpha".into(), "beta".into()],
            None,
        )
        .await
        .expect("insert both");
        insert_memory(&pool, "stub", "no tags", &emb, "stub", "test", &[], None)
            .await
            .expect("insert none");

        let query = make_embedding(1.0);
        let alpha_results = search_memories(&pool, "stub", &query, Some(&["alpha".into()]), 10)
            .await
            .expect("search with alpha tag");
        assert_eq!(alpha_results.len(), 2); // "tagged a" and "tagged both"

        let beta_results = search_memories(&pool, "stub", &query, Some(&["beta".into()]), 10)
            .await
            .expect("search with beta tag");
        assert_eq!(beta_results.len(), 2); // "tagged b" and "tagged both"

        // Empty filter returns all
        let all_results = search_memories(&pool, "stub", &query, Some(&[]), 10)
            .await
            .expect("search with empty filter");
        assert_eq!(all_results.len(), 4);
    }

    #[tokio::test]
    async fn test_memory_link() {
        let pool = open_in_memory_named("mem_test_link")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "stub", 384)
            .await
            .expect("create table");
        let emb = make_embedding(0.0);

        let mem1 = insert_memory(&pool, "stub", "memory 1", &emb, "stub", "test", &[], None)
            .await
            .expect("insert 1");
        let mem2 = insert_memory(&pool, "stub", "memory 2", &emb, "stub", "test", &[], None)
            .await
            .expect("insert 2");

        insert_memory_link(&pool, &mem1.id, &mem2.id, "memory", "related")
            .await
            .expect("link insert succeeds");

        // Verify the link exists
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_links WHERE memory_id = ?1 AND linked_id = ?2",
        )
        .bind(&mem1.id)
        .bind(&mem2.id)
        .fetch_one(&pool)
        .await
        .expect("count query");

        assert_eq!(count, 1);
    }
}
