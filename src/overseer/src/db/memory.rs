use sqlx::{Row, SqlitePool};
use uuid::Uuid;
use zerocopy::IntoBytes;

use crate::error::{OverseerError, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub embedding_model: String,
    pub source: String,
    pub tags: Vec<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, serde::Serialize)]
pub struct MemorySearchResult {
    pub memory: Memory,
    pub distance: f64,
}

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

pub async fn insert_memory(
    pool: &SqlitePool,
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

    // Insert into memories table and get the rowid
    let rowid: i64 = sqlx::query_scalar(
        "INSERT INTO memories (id, content, embedding_model, source, tags, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         RETURNING rowid",
    )
    .bind(&id)
    .bind(content)
    .bind(embedding_model)
    .bind(source)
    .bind(&tags_json)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .map_err(OverseerError::Storage)?;

    // Insert into memory_embeddings virtual table using the same rowid
    let embedding_bytes: &[u8] = embedding.as_bytes();
    sqlx::query("INSERT INTO memory_embeddings (rowid, embedding) VALUES (?1, ?2)")
        .bind(rowid)
        .bind(embedding_bytes)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    get_memory(pool, &id).await
}

pub async fn get_memory(pool: &SqlitePool, id: &str) -> Result<Memory> {
    let row = sqlx::query(
        "SELECT id, content, embedding_model, source, tags, expires_at, created_at, updated_at \
         FROM memories WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(OverseerError::Storage)?
    .ok_or_else(|| OverseerError::NotFound(format!("memory {id}")))?;

    Ok(row_to_memory(&row))
}

pub async fn delete_memory(pool: &SqlitePool, id: &str) -> Result<()> {
    // Delete from memories table. Orphaned embeddings in the vec0 virtual table
    // are harmless since search_memories joins with memories, which filters them out.
    // (Deleting from vec0 tables has quirks with the validity blob in in-memory DBs)
    sqlx::query("DELETE FROM memories WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(())
}

pub async fn search_memories(
    pool: &SqlitePool,
    query_embedding: &[f32],
    tags_filter: Option<&[String]>,
    limit: usize,
) -> Result<Vec<MemorySearchResult>> {
    let embedding_bytes: &[u8] = query_embedding.as_bytes();
    // Fetch more than `limit` so we have room to post-filter by tags
    let fetch_limit = (limit * 10).max(100) as i64;

    let rows = sqlx::query(
        "SELECT m.id, m.content, m.embedding_model, m.source, m.tags, m.expires_at, \
         m.created_at, m.updated_at, v.distance \
         FROM memory_embeddings v \
         JOIN memories m ON m.rowid = v.rowid \
         WHERE v.embedding MATCH ?1 AND k = ?2 \
         ORDER BY v.distance",
    )
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

    // Post-filter by tags if requested
    if let Some(filter_tags) = tags_filter {
        if !filter_tags.is_empty() {
            results.retain(|r| filter_tags.iter().any(|ft| r.memory.tags.contains(ft)));
        }
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
    sqlx::query(
        "INSERT INTO memory_links (memory_id, linked_id, linked_type, relation_type) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(memory_id)
    .bind(linked_id)
    .bind(linked_type)
    .bind(relation_type)
    .execute(pool)
    .await
    .map_err(OverseerError::Storage)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn make_embedding(val: f32) -> Vec<f32> {
        vec![val; 384]
    }

    #[tokio::test]
    async fn test_insert_and_get_memory() {
        let pool = open_in_memory().await.expect("pool opens");
        let tags = vec!["test".to_string(), "demo".to_string()];
        let embedding = make_embedding(0.1);

        let memory = insert_memory(
            &pool,
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
        let pool = open_in_memory().await.expect("pool opens");
        let embedding = make_embedding(0.0);

        let memory = insert_memory(
            &pool,
            "to be deleted",
            &embedding,
            "stub",
            "unit-test",
            &[],
            None,
        )
        .await
        .expect("insert succeeds");

        delete_memory(&pool, &memory.id)
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
        let pool = open_in_memory().await.expect("pool opens");

        // Insert 3 memories with distinct embeddings
        let emb_a = make_embedding(1.0);
        let emb_b = make_embedding(0.5);
        let emb_c = make_embedding(0.1);

        let mem_a = insert_memory(&pool, "close one", &emb_a, "stub", "test", &[], None)
            .await
            .expect("insert a");
        let _mem_b = insert_memory(&pool, "middle one", &emb_b, "stub", "test", &[], None)
            .await
            .expect("insert b");
        let _mem_c = insert_memory(&pool, "far one", &emb_c, "stub", "test", &[], None)
            .await
            .expect("insert c");

        // Query with embedding closest to emb_a
        let query = make_embedding(1.0);
        let results = search_memories(&pool, &query, None, 3)
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
    async fn test_memory_link() {
        let pool = open_in_memory().await.expect("pool opens");
        let emb = make_embedding(0.0);

        let mem1 = insert_memory(&pool, "memory 1", &emb, "stub", "test", &[], None)
            .await
            .expect("insert 1");
        let mem2 = insert_memory(&pool, "memory 2", &emb, "stub", "test", &[], None)
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
