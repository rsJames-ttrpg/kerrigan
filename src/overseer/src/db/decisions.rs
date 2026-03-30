use chrono::NaiveDateTime;
use sea_query::{Expr, Order, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub use super::models::Decision;
use super::tables::Decisions;
use crate::error::{OverseerError, Result};

fn row_to_decision(row: &sqlx::sqlite::SqliteRow) -> Decision {
    let tags_json: String = row.get("tags");
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_else(|e| {
        tracing::warn!(id = %row.get::<String, _>("id"), error = %e, "failed to deserialize tags, defaulting to empty");
        Vec::new()
    });
    Decision {
        id: row.get("id"),
        agent: row.get("agent"),
        context: row.get("context"),
        decision: row.get("decision"),
        reasoning: row.get("reasoning"),
        tags,
        run_id: row.get("run_id"),
        created_at: row.get::<NaiveDateTime, _>("created_at").and_utc(),
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

    let (sql, values) = Query::insert()
        .into_table(Decisions::Table)
        .columns([
            Decisions::Id,
            Decisions::Agent,
            Decisions::Context,
            Decisions::Decision,
            Decisions::Reasoning,
            Decisions::Tags,
            Decisions::RunId,
        ])
        .values_panic([
            id.into(),
            agent.into(),
            context.into(),
            decision.into(),
            reasoning.into(),
            tags_json.into(),
            run_id.map(|s| s.to_string()).into(),
        ])
        .returning(Query::returning().columns([
            Decisions::Id,
            Decisions::Agent,
            Decisions::Context,
            Decisions::Decision,
            Decisions::Reasoning,
            Decisions::Tags,
            Decisions::RunId,
            Decisions::CreatedAt,
        ]))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_decision(&row))
}

pub async fn get_decision(pool: &SqlitePool, id: &str) -> Result<Option<Decision>> {
    let (sql, values) = Query::select()
        .columns([
            Decisions::Id,
            Decisions::Agent,
            Decisions::Context,
            Decisions::Decision,
            Decisions::Reasoning,
            Decisions::Tags,
            Decisions::RunId,
            Decisions::CreatedAt,
        ])
        .from(Decisions::Table)
        .and_where(Expr::col(Decisions::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
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
    // Over-fetch to compensate for post-filtering by tags
    let fetch_limit = if tags.is_some_and(|t| !t.is_empty()) {
        (limit * 10).max(100)
    } else {
        limit
    };

    let mut query = Query::select();
    query
        .columns([
            Decisions::Id,
            Decisions::Agent,
            Decisions::Context,
            Decisions::Decision,
            Decisions::Reasoning,
            Decisions::Tags,
            Decisions::RunId,
            Decisions::CreatedAt,
        ])
        .from(Decisions::Table);

    if let Some(a) = agent {
        query.and_where(Expr::col(Decisions::Agent).eq(a));
    }

    query
        .order_by(Decisions::CreatedAt, Order::Desc)
        .limit(fetch_limit as u64);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    let mut results: Vec<Decision> = rows.iter().map(row_to_decision).collect();

    if let Some(filter_tags) = tags
        && !filter_tags.is_empty()
    {
        results.retain(|d| filter_tags.iter().any(|ft| d.tags.contains(ft)));
    }

    results.truncate(limit as usize);
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
    async fn test_query_decisions_limit_with_tag_filter() {
        let pool = open_in_memory_named("decisions_test_limit_tags")
            .await
            .expect("pool opens");

        let target_tag = vec!["target".to_string()];
        let other_tag = vec!["other".to_string()];

        // Insert 5 with target tag, 20 with other tag
        for i in 0..5 {
            log_decision(
                &pool,
                "agent",
                "ctx",
                &format!("target-{i}"),
                "r",
                &target_tag,
                None,
            )
            .await
            .expect("log target");
        }
        for i in 0..20 {
            log_decision(
                &pool,
                "agent",
                "ctx",
                &format!("other-{i}"),
                "r",
                &other_tag,
                None,
            )
            .await
            .expect("log other");
        }

        // Query with limit=3 and tag filter — should get exactly 3
        let results = query_decisions(&pool, None, Some(&target_tag), 3)
            .await
            .expect("query succeeds");
        assert_eq!(results.len(), 3);
        assert!(
            results
                .iter()
                .all(|d| d.tags.contains(&"target".to_string()))
        );

        // Query with limit=10 and tag filter — only 5 exist, should get 5
        let results = query_decisions(&pool, None, Some(&target_tag), 10)
            .await
            .expect("query succeeds");
        assert_eq!(results.len(), 5);
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
