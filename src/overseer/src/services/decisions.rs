use sqlx::SqlitePool;

use crate::db::decisions as db;
use crate::error::Result;

pub struct DecisionService {
    pool: SqlitePool,
}

impl DecisionService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn log(
        &self,
        agent: &str,
        context: &str,
        decision: &str,
        reasoning: &str,
        tags: &[String],
        run_id: Option<&str>,
    ) -> Result<db::Decision> {
        db::log_decision(
            &self.pool, agent, context, decision, reasoning, tags, run_id,
        )
        .await
    }

    pub async fn query(
        &self,
        agent: Option<&str>,
        tags: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<db::Decision>> {
        db::query_decisions(&self.pool, agent, tags, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_named;

    #[tokio::test]
    async fn test_decision_service_log_and_query() {
        let pool = open_in_memory_named("svc_decisions_test_log")
            .await
            .expect("pool opens");
        let svc = DecisionService::new(pool);

        let tags = vec!["routing".to_string()];
        let dec = svc
            .log(
                "agent-svc-dec-1",
                "user asked X",
                "do Y",
                "because Z",
                &tags,
                None,
            )
            .await
            .expect("log succeeds");

        assert_eq!(dec.agent, "agent-svc-dec-1");
        assert_eq!(dec.tags, tags);

        let results = svc
            .query(Some("agent-svc-dec-1"), None, 10)
            .await
            .expect("query succeeds");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, dec.id);

        let empty = svc
            .query(Some("agent-svc-dec-2"), None, 10)
            .await
            .expect("query empty");
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn test_decision_service_query_by_tags() {
        let pool = open_in_memory_named("svc_decisions_test_tags")
            .await
            .expect("pool opens");
        let svc = DecisionService::new(pool);

        svc.log("a", "c", "d", "r", &["foo-svc".to_string()], None)
            .await
            .expect("log 1");
        svc.log("a", "c", "d", "r", &["bar-svc".to_string()], None)
            .await
            .expect("log 2");

        let foo_results = svc
            .query(None, Some(&["foo-svc".to_string()]), 10)
            .await
            .expect("query by foo");
        assert_eq!(foo_results.len(), 1);
    }
}
