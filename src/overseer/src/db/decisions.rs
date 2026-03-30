use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::{OverseerError, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Decision {
    pub id: String,
    pub agent: String,
    pub context: String,
    pub decision: String,
    pub reasoning: String,
    pub tags: Vec<String>,
    pub run_id: Option<String>,
    pub created_at: String,
}

fn row_to_decision(row: &sqlx::sqlite::SqliteRow) -> Decision {
    let tags_json: String = row.get("tags");
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    Decision {
        id: row.get("id"),
        agent: row.get("agent"),
        context: row.get("context"),
        decision: row.get("decision"),
        reasoning: row.get("reasoning"),
        tags,
        run_id: row.get("run_id"),
        created_at: row.get("created_at"),
    }
}

pub async fn log_decision(
    pool: &SqlitePool,
    agent: &str,
    context: &str,
    decision: &str,
    reasoning: &str,
    tags: &[String],
    run_id: Option<&str>,
) -> Result<Decision> {
    let id = Uuid::new_v4().to_string();
    let tags_json =
        serde_json::to_string(tags).map_err(|e| OverseerError::Internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO decisions (id, agent, context, decision, reasoning, tags, run_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(&id)
    .bind(agent)
    .bind(context)
    .bind(decision)
    .bind(reasoning)
    .bind(&tags_json)
    .bind(run_id)
    .execute(pool)
    .await
    .map_err(OverseerError::Storage)?;

    get_decision(pool, &id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("decision {id}")))
}

pub async fn get_decision(pool: &SqlitePool, id: &str) -> Result<Option<Decision>> {
    let row = sqlx::query(
        "SELECT id, agent, context, decision, reasoning, tags, run_id, created_at \
         FROM decisions WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_decision))
}

pub async fn query_decisions(
    pool: &SqlitePool,
    agent: Option<&str>,
    tags: Option<&[String]>,
    limit: i64,
) -> Result<Vec<Decision>> {
    let rows = sqlx::query(
        "SELECT id, agent, context, decision, reasoning, tags, run_id, created_at \
         FROM decisions \
         WHERE (?1 IS NULL OR agent = ?1) \
         ORDER BY created_at DESC \
         LIMIT ?2",
    )
    .bind(agent)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(OverseerError::Storage)?;

    let mut results: Vec<Decision> = rows.iter().map(row_to_decision).collect();

    // Post-filter by tags in Rust
    if let Some(filter_tags) = tags {
        if !filter_tags.is_empty() {
            results.retain(|d| filter_tags.iter().any(|ft| d.tags.contains(ft)));
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_named;

    #[tokio::test]
    async fn test_log_and_get_decision() {
        let pool = open_in_memory_named("decisions_test_log_get")
            .await
            .expect("pool opens");

        let tags = vec!["important".to_string(), "routing".to_string()];
        let dec = log_decision(
            &pool,
            "agent-1",
            "user asked about weather",
            "use weather tool",
            "the user's query contains weather-related keywords",
            &tags,
            None,
        )
        .await
        .expect("log succeeds");

        assert!(!dec.id.is_empty());
        assert_eq!(dec.agent, "agent-1");
        assert_eq!(dec.decision, "use weather tool");
        assert_eq!(dec.tags, tags);

        let fetched = get_decision(&pool, &dec.id)
            .await
            .expect("get succeeds")
            .expect("decision exists");
        assert_eq!(fetched.id, dec.id);
        assert_eq!(fetched.agent, dec.agent);
    }

    #[tokio::test]
    async fn test_query_by_agent() {
        let pool = open_in_memory_named("decisions_test_query_agent")
            .await
            .expect("pool opens");

        log_decision(&pool, "agent-qba-1", "ctx", "dec1", "r", &[], None)
            .await
            .expect("log 1");
        log_decision(&pool, "agent-qba-2", "ctx", "dec2", "r", &[], None)
            .await
            .expect("log 2");
        log_decision(&pool, "agent-qba-1", "ctx", "dec3", "r", &[], None)
            .await
            .expect("log 3");

        let agent1_results = query_decisions(&pool, Some("agent-qba-1"), None, 100)
            .await
            .expect("query succeeds");
        assert_eq!(agent1_results.len(), 2);
        assert!(agent1_results.iter().all(|d| d.agent == "agent-qba-1"));

        let all = query_decisions(&pool, None, None, 100)
            .await
            .expect("query all");
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn test_query_by_tags() {
        let pool = open_in_memory_named("decisions_test_query_tags")
            .await
            .expect("pool opens");

        let tags_a = vec!["alpha".to_string()];
        let tags_b = vec!["beta".to_string()];
        let tags_both = vec!["alpha".to_string(), "beta".to_string()];

        log_decision(&pool, "agent-1", "ctx", "dec1", "r", &tags_a, None)
            .await
            .expect("log 1");
        log_decision(&pool, "agent-1", "ctx", "dec2", "r", &tags_b, None)
            .await
            .expect("log 2");
        log_decision(&pool, "agent-1", "ctx", "dec3", "r", &tags_both, None)
            .await
            .expect("log 3");

        let alpha_results = query_decisions(&pool, None, Some(&tags_a), 100)
            .await
            .expect("query by alpha");
        // dec1 and dec3 have alpha tag
        assert_eq!(alpha_results.len(), 2);

        let beta_results = query_decisions(&pool, None, Some(&tags_b), 100)
            .await
            .expect("query by beta");
        // dec2 and dec3 have beta tag
        assert_eq!(beta_results.len(), 2);
    }

    #[tokio::test]
    async fn test_query_empty_results() {
        let pool = open_in_memory_named("decisions_test_empty")
            .await
            .expect("pool opens");

        let results = query_decisions(&pool, Some("nonexistent-agent"), None, 100)
            .await
            .expect("query succeeds");
        assert!(results.is_empty());
    }
}
