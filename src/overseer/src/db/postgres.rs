use async_trait::async_trait;
use sea_query::{Expr, Order, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::models::*;
use super::tables::*;
use super::trait_def::{ArtifactStore, DecisionStore, HatcheryStore, JobStore, MemoryStore};
use crate::error::{OverseerError, Result};

/// A PostgreSQL-backed implementation of the `Database` trait.
pub struct PostgresDatabase {
    pool: PgPool,
}

impl PostgresDatabase {
    pub async fn open(url: &str) -> std::result::Result<Self, OverseerError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .map_err(OverseerError::Storage)?;

        sqlx::migrate!("migrations/postgres")
            .run(&pool)
            .await
            .map_err(|e| OverseerError::Storage(e.into()))?;

        Ok(Self { pool })
    }
}

// ── Row mappers ──────────────────────────────────────────────────────────────

fn row_to_memory(row: &sqlx::postgres::PgRow) -> Memory {
    let tags_json: serde_json::Value = row.get("tags");
    let tags: Vec<String> = serde_json::from_value(tags_json).unwrap_or_else(|e| {
        tracing::warn!(id = %row.get::<String, _>("id"), error = %e, "failed to deserialize tags, defaulting to empty");
        Vec::new()
    });
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

fn row_to_job_definition(row: &sqlx::postgres::PgRow) -> JobDefinition {
    let config: serde_json::Value = row.get("config");
    JobDefinition {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        config,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_job_run(row: &sqlx::postgres::PgRow) -> JobRun {
    let result: Option<serde_json::Value> = row.get("result");
    JobRun {
        id: row.get("id"),
        definition_id: row.get("definition_id"),
        parent_id: row.get("parent_id"),
        status: row
            .get::<String, _>("status")
            .parse()
            .unwrap_or(JobRunStatus::Pending),
        triggered_by: row.get("triggered_by"),
        config_overrides: row.get("config_overrides"),
        result,
        error: row.get("error"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
    }
}

fn row_to_task(row: &sqlx::postgres::PgRow) -> Task {
    let output: Option<serde_json::Value> = row.get("output");
    Task {
        id: row.get("id"),
        run_id: row.get("run_id"),
        subject: row.get("subject"),
        status: row
            .get::<String, _>("status")
            .parse()
            .unwrap_or(TaskStatus::Pending),
        assigned_to: row.get("assigned_to"),
        output,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_decision(row: &sqlx::postgres::PgRow) -> Decision {
    let tags_json: serde_json::Value = row.get("tags");
    let tags: Vec<String> = serde_json::from_value(tags_json).unwrap_or_else(|e| {
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
        created_at: row.get("created_at"),
    }
}

fn row_to_artifact(row: &sqlx::postgres::PgRow) -> ArtifactMetadata {
    ArtifactMetadata {
        id: row.get("id"),
        name: row.get("name"),
        content_type: row.get("content_type"),
        size: row.get("size"),
        run_id: row.get("run_id"),
        created_at: row.get("created_at"),
    }
}

// ── Database trait implementation ────────────────────────────────────────────

#[async_trait]
impl MemoryStore for PostgresDatabase {
    async fn create_embedding_table(&self, provider_name: &str, _dimensions: usize) -> Result<()> {
        // The memory_embeddings table is created by migration; create a partial HNSW index
        // for this provider to accelerate vector search. If creation fails (e.g. mixed
        // dimensions in existing data), warn and fall back to sequential scan.
        let index_sql = format!(
            "CREATE INDEX IF NOT EXISTS idx_embeddings_{provider_name} \
             ON memory_embeddings USING hnsw (embedding vector_l2_ops) \
             WHERE provider = '{provider_name}'"
        );
        if let Err(e) = sqlx::raw_sql(&index_sql).execute(&self.pool).await {
            tracing::warn!(
                provider = %provider_name,
                error = %e,
                "failed to create HNSW index — vector search will use sequential scan"
            );
        }
        Ok(())
    }

    async fn insert_memory(
        &self,
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
            serde_json::to_value(tags).map_err(|e| OverseerError::Internal(e.to_string()))?;
        let tags_str = serde_json::to_string(&tags_json)
            .map_err(|e| OverseerError::Internal(e.to_string()))?;

        let mut tx = self.pool.begin().await.map_err(OverseerError::Storage)?;

        // Insert into memories table
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
            .values([
                id.clone().into(),
                content.into(),
                embedding_model.into(),
                source.into(),
                tags_str.into(),
                expires_at.map(|s| s.to_string()).into(),
            ])
            .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
            .build_sqlx(PostgresQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(&mut *tx)
            .await
            .map_err(OverseerError::Storage)?;

        // Insert embedding with pgvector
        let vec = pgvector::Vector::from(embedding.to_vec());
        sqlx::query(
            "INSERT INTO memory_embeddings (memory_id, provider, embedding) VALUES ($1, $2, $3)",
        )
        .bind(&id)
        .bind(provider_name)
        .bind(&vec)
        .execute(&mut *tx)
        .await
        .map_err(OverseerError::Storage)?;

        tx.commit().await.map_err(OverseerError::Storage)?;

        self.get_memory(&id)
            .await?
            .ok_or_else(|| OverseerError::Internal("memory not found after insert".to_string()))
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
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
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row.as_ref().map(row_to_memory))
    }

    async fn delete_memory(&self, _provider_name: &str, id: &str) -> Result<()> {
        // ON DELETE CASCADE handles embedding cleanup
        let (sql, values) = Query::delete()
            .from_table(Memories::Table)
            .and_where(Expr::col(Memories::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(())
    }

    async fn search_memories(
        &self,
        provider_name: &str,
        query_embedding: &[f32],
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>> {
        let vec = pgvector::Vector::from(query_embedding.to_vec());
        let limit_i64 = limit as i64;

        let rows = if let Some(tags) = tags_filter
            && !tags.is_empty()
        {
            // Use JSONB ?| operator: matches rows where tags array contains ANY of the given values
            sqlx::query(
                "SELECT m.id, m.content, m.embedding_model, m.source, m.tags, m.expires_at, \
                 m.created_at, m.updated_at, e.embedding <-> $1 AS distance \
                 FROM memory_embeddings e \
                 JOIN memories m ON m.id = e.memory_id \
                 WHERE e.provider = $2 AND m.tags ?| $4 \
                 ORDER BY e.embedding <-> $1 \
                 LIMIT $3",
            )
            .bind(&vec)
            .bind(provider_name)
            .bind(limit_i64)
            .bind(tags)
            .fetch_all(&self.pool)
            .await
            .map_err(OverseerError::Storage)?
        } else {
            sqlx::query(
                "SELECT m.id, m.content, m.embedding_model, m.source, m.tags, m.expires_at, \
                 m.created_at, m.updated_at, e.embedding <-> $1 AS distance \
                 FROM memory_embeddings e \
                 JOIN memories m ON m.id = e.memory_id \
                 WHERE e.provider = $2 \
                 ORDER BY e.embedding <-> $1 \
                 LIMIT $3",
            )
            .bind(&vec)
            .bind(provider_name)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await
            .map_err(OverseerError::Storage)?
        };

        let results: Vec<MemorySearchResult> = rows
            .iter()
            .map(|row| {
                let distance: f64 = row.get("distance");
                MemorySearchResult {
                    memory: row_to_memory(row),
                    distance,
                }
            })
            .collect();

        Ok(results)
    }

    async fn insert_memory_link(
        &self,
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
            .values([
                memory_id.into(),
                linked_id.into(),
                linked_type.into(),
                relation_type.into(),
            ])
            .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
            .build_sqlx(PostgresQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(())
    }
}

#[async_trait]
impl JobStore for PostgresDatabase {
    async fn create_job_definition(
        &self,
        name: &str,
        description: &str,
        config: serde_json::Value,
    ) -> Result<JobDefinition> {
        let id = Uuid::new_v4().to_string();
        let config_str =
            serde_json::to_string(&config).map_err(|e| OverseerError::Internal(e.to_string()))?;

        let (sql, values) = Query::insert()
            .into_table(JobDefinitions::Table)
            .columns([
                JobDefinitions::Id,
                JobDefinitions::Name,
                JobDefinitions::Description,
                JobDefinitions::Config,
            ])
            .values([
                id.into(),
                name.into(),
                description.into(),
                config_str.into(),
            ])
            .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
            .returning(Query::returning().columns([
                JobDefinitions::Id,
                JobDefinitions::Name,
                JobDefinitions::Description,
                JobDefinitions::Config,
                JobDefinitions::CreatedAt,
                JobDefinitions::UpdatedAt,
            ]))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_one(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row_to_job_definition(&row))
    }

    async fn get_job_definition(&self, id: &str) -> Result<Option<JobDefinition>> {
        let (sql, values) = Query::select()
            .columns([
                JobDefinitions::Id,
                JobDefinitions::Name,
                JobDefinitions::Description,
                JobDefinitions::Config,
                JobDefinitions::CreatedAt,
                JobDefinitions::UpdatedAt,
            ])
            .from(JobDefinitions::Table)
            .and_where(Expr::col(JobDefinitions::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row.as_ref().map(row_to_job_definition))
    }

    async fn list_job_definitions(&self) -> Result<Vec<JobDefinition>> {
        let (sql, values) = Query::select()
            .columns([
                JobDefinitions::Id,
                JobDefinitions::Name,
                JobDefinitions::Description,
                JobDefinitions::Config,
                JobDefinitions::CreatedAt,
                JobDefinitions::UpdatedAt,
            ])
            .from(JobDefinitions::Table)
            .order_by(JobDefinitions::CreatedAt, Order::Asc)
            .build_sqlx(PostgresQueryBuilder);

        let rows = sqlx::query_with(&sql, values)
            .fetch_all(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(rows.iter().map(row_to_job_definition).collect())
    }

    async fn start_job_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
        config_overrides: Option<serde_json::Value>,
    ) -> Result<JobRun> {
        let id = Uuid::new_v4().to_string();
        let config_overrides_json = config_overrides
            .as_ref()
            .map(|v| serde_json::to_string(v).map_err(|e| OverseerError::Internal(e.to_string())))
            .transpose()?;

        let (sql, values) = Query::insert()
            .into_table(JobRuns::Table)
            .columns([
                JobRuns::Id,
                JobRuns::DefinitionId,
                JobRuns::ParentId,
                JobRuns::Status,
                JobRuns::TriggeredBy,
                JobRuns::ConfigOverrides,
                JobRuns::StartedAt,
            ])
            .values([
                id.into(),
                definition_id.into(),
                parent_id.map(|s| s.to_string()).into(),
                "pending".into(),
                triggered_by.into(),
                config_overrides_json.into(),
                Expr::cust("now()"),
            ])
            .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
            .returning(Query::returning().columns([
                JobRuns::Id,
                JobRuns::DefinitionId,
                JobRuns::ParentId,
                JobRuns::Status,
                JobRuns::TriggeredBy,
                JobRuns::ConfigOverrides,
                JobRuns::Result,
                JobRuns::Error,
                JobRuns::StartedAt,
                JobRuns::CompletedAt,
            ]))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_one(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row_to_job_run(&row))
    }

    async fn get_job_run(&self, id: &str) -> Result<Option<JobRun>> {
        let (sql, values) = Query::select()
            .columns([
                JobRuns::Id,
                JobRuns::DefinitionId,
                JobRuns::ParentId,
                JobRuns::Status,
                JobRuns::TriggeredBy,
                JobRuns::ConfigOverrides,
                JobRuns::Result,
                JobRuns::Error,
                JobRuns::StartedAt,
                JobRuns::CompletedAt,
            ])
            .from(JobRuns::Table)
            .and_where(Expr::col(JobRuns::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row.as_ref().map(row_to_job_run))
    }

    async fn update_job_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> Result<JobRun> {
        let result_str = result
            .as_ref()
            .map(|v| serde_json::to_string(v).map_err(|e| OverseerError::Internal(e.to_string())))
            .transpose()?;

        let parsed_status = status
            .map(|s| s.parse::<JobRunStatus>())
            .transpose()
            .map_err(OverseerError::Validation)?;
        let is_terminal = parsed_status
            .as_ref()
            .map(|s| s.is_terminal())
            .unwrap_or(false);

        if status.is_none() && result.is_none() && error.is_none() {
            // Nothing to update — just return the current state
            return self
                .get_job_run(id)
                .await?
                .ok_or_else(|| OverseerError::NotFound(format!("job_run {id}")));
        }

        let mut query = Query::update();
        query.table(JobRuns::Table);

        if let Some(ref s) = parsed_status {
            query.value(JobRuns::Status, s.to_string());
        }
        if let Some(ref r) = result_str {
            query.value(JobRuns::Result, r.as_str());
        }
        if let Some(e) = error {
            query.value(JobRuns::Error, e);
        }
        if is_terminal {
            query.value(JobRuns::CompletedAt, Expr::cust("now()"));
        }

        query.and_where(Expr::col(JobRuns::Id).eq(id));

        let (sql, values) = query.build_sqlx(PostgresQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        self.get_job_run(id)
            .await?
            .ok_or_else(|| OverseerError::NotFound(format!("job_run {id}")))
    }

    async fn list_job_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>> {
        let mut query = Query::select();
        query
            .columns([
                JobRuns::Id,
                JobRuns::DefinitionId,
                JobRuns::ParentId,
                JobRuns::Status,
                JobRuns::TriggeredBy,
                JobRuns::ConfigOverrides,
                JobRuns::Result,
                JobRuns::Error,
                JobRuns::StartedAt,
                JobRuns::CompletedAt,
            ])
            .from(JobRuns::Table);

        if let Some(s) = status {
            query.and_where(Expr::col(JobRuns::Status).eq(s));
        }

        query.order_by(JobRuns::StartedAt, Order::Asc);

        let (sql, values) = query.build_sqlx(PostgresQueryBuilder);

        let rows = sqlx::query_with(&sql, values)
            .fetch_all(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(rows.iter().map(row_to_job_run).collect())
    }

    async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Task> {
        let id = Uuid::new_v4().to_string();

        let (sql, values) = Query::insert()
            .into_table(Tasks::Table)
            .columns([Tasks::Id, Tasks::Subject, Tasks::RunId, Tasks::AssignedTo])
            .values([
                id.into(),
                subject.into(),
                run_id.map(|s| s.to_string()).into(),
                assigned_to.map(|s| s.to_string()).into(),
            ])
            .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
            .returning(Query::returning().columns([
                Tasks::Id,
                Tasks::RunId,
                Tasks::Subject,
                Tasks::Status,
                Tasks::AssignedTo,
                Tasks::Output,
                Tasks::CreatedAt,
                Tasks::UpdatedAt,
            ]))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_one(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row_to_task(&row))
    }

    async fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let (sql, values) = Query::select()
            .columns([
                Tasks::Id,
                Tasks::RunId,
                Tasks::Subject,
                Tasks::Status,
                Tasks::AssignedTo,
                Tasks::Output,
                Tasks::CreatedAt,
                Tasks::UpdatedAt,
            ])
            .from(Tasks::Table)
            .and_where(Expr::col(Tasks::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row.as_ref().map(row_to_task))
    }

    async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<Task> {
        let output_str = output
            .as_ref()
            .map(|v| serde_json::to_string(v).map_err(|e| OverseerError::Internal(e.to_string())))
            .transpose()?;

        let mut query = Query::update();
        query.table(Tasks::Table);

        if let Some(s) = status {
            query.value(Tasks::Status, s);
        }
        if let Some(a) = assigned_to {
            query.value(Tasks::AssignedTo, a);
        }
        if let Some(ref o) = output_str {
            query.value(Tasks::Output, o.as_str());
        }

        query.value(Tasks::UpdatedAt, Expr::cust("now()"));
        query.and_where(Expr::col(Tasks::Id).eq(id));

        let (sql, values) = query.build_sqlx(PostgresQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        self.get_task(id)
            .await?
            .ok_or_else(|| OverseerError::NotFound(format!("task {id}")))
    }

    async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<Task>> {
        let mut query = Query::select();
        query
            .columns([
                Tasks::Id,
                Tasks::RunId,
                Tasks::Subject,
                Tasks::Status,
                Tasks::AssignedTo,
                Tasks::Output,
                Tasks::CreatedAt,
                Tasks::UpdatedAt,
            ])
            .from(Tasks::Table);

        if let Some(s) = status {
            query.and_where(Expr::col(Tasks::Status).eq(s));
        }
        if let Some(a) = assigned_to {
            query.and_where(Expr::col(Tasks::AssignedTo).eq(a));
        }
        if let Some(r) = run_id {
            query.and_where(Expr::col(Tasks::RunId).eq(r));
        }

        query.order_by(Tasks::CreatedAt, Order::Asc);

        let (sql, values) = query.build_sqlx(PostgresQueryBuilder);

        let rows = sqlx::query_with(&sql, values)
            .fetch_all(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(rows.iter().map(row_to_task).collect())
    }
}

#[async_trait]
impl DecisionStore for PostgresDatabase {
    async fn log_decision(
        &self,
        agent: &str,
        context: &str,
        decision: &str,
        reasoning: &str,
        tags: &[String],
        run_id: Option<&str>,
    ) -> Result<Decision> {
        let id = Uuid::new_v4().to_string();
        let tags_str =
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
            .values([
                id.into(),
                agent.into(),
                context.into(),
                decision.into(),
                reasoning.into(),
                tags_str.into(),
                run_id.map(|s| s.to_string()).into(),
            ])
            .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
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
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_one(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row_to_decision(&row))
    }

    async fn get_decision(&self, id: &str) -> Result<Option<Decision>> {
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
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row.as_ref().map(row_to_decision))
    }

    async fn query_decisions(
        &self,
        agent: Option<&str>,
        tags: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<Decision>> {
        let filter_tags = tags.filter(|t| !t.is_empty());

        let rows = match (agent, filter_tags) {
            (Some(a), Some(t)) => {
                // Both agent and tags filters
                sqlx::query(
                    "SELECT id, agent, context, decision, reasoning, tags, run_id, created_at \
                     FROM decisions \
                     WHERE agent = $1 AND tags ?| $2 \
                     ORDER BY created_at DESC \
                     LIMIT $3",
                )
                .bind(a)
                .bind(t)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .map_err(OverseerError::Storage)?
            }
            (Some(a), None) => {
                // Agent filter only
                sqlx::query(
                    "SELECT id, agent, context, decision, reasoning, tags, run_id, created_at \
                     FROM decisions \
                     WHERE agent = $1 \
                     ORDER BY created_at DESC \
                     LIMIT $2",
                )
                .bind(a)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .map_err(OverseerError::Storage)?
            }
            (None, Some(t)) => {
                // Tags filter only
                sqlx::query(
                    "SELECT id, agent, context, decision, reasoning, tags, run_id, created_at \
                     FROM decisions \
                     WHERE tags ?| $1 \
                     ORDER BY created_at DESC \
                     LIMIT $2",
                )
                .bind(t)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .map_err(OverseerError::Storage)?
            }
            (None, None) => {
                // No filters
                sqlx::query(
                    "SELECT id, agent, context, decision, reasoning, tags, run_id, created_at \
                     FROM decisions \
                     ORDER BY created_at DESC \
                     LIMIT $1",
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .map_err(OverseerError::Storage)?
            }
        };

        Ok(rows.iter().map(row_to_decision).collect())
    }
}

#[async_trait]
impl ArtifactStore for PostgresDatabase {
    async fn insert_artifact(
        &self,
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
            .values([
                id.into(),
                name.into(),
                content_type.into(),
                size.into(),
                run_id.map(|s| s.to_string()).into(),
            ])
            .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
            .returning(Query::returning().columns([
                Artifacts::Id,
                Artifacts::Name,
                Artifacts::ContentType,
                Artifacts::Size,
                Artifacts::RunId,
                Artifacts::CreatedAt,
            ]))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_one(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row_to_artifact(&row))
    }

    async fn get_artifact(&self, id: &str) -> Result<Option<ArtifactMetadata>> {
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
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row.as_ref().map(row_to_artifact))
    }

    async fn list_artifacts(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>> {
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

        let (sql, values) = query.build_sqlx(PostgresQueryBuilder);

        let rows = sqlx::query_with(&sql, values)
            .fetch_all(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(rows.iter().map(row_to_artifact).collect())
    }
}

#[async_trait]
impl HatcheryStore for PostgresDatabase {
    async fn register_hatchery(
        &self,
        _name: &str,
        _capabilities: serde_json::Value,
        _max_concurrency: i32,
    ) -> crate::error::Result<Hatchery> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }

    async fn get_hatchery(&self, _id: &str) -> crate::error::Result<Option<Hatchery>> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }

    async fn get_hatchery_by_name(&self, _name: &str) -> crate::error::Result<Option<Hatchery>> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }

    async fn heartbeat_hatchery(
        &self,
        _id: &str,
        _status: &str,
        _active_drones: i32,
    ) -> crate::error::Result<Hatchery> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }

    async fn list_hatcheries(&self, _status: Option<&str>) -> crate::error::Result<Vec<Hatchery>> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }

    async fn deregister_hatchery(&self, _id: &str) -> crate::error::Result<()> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }

    async fn assign_job_to_hatchery(
        &self,
        _job_run_id: &str,
        _hatchery_id: &str,
    ) -> crate::error::Result<JobRun> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }

    async fn list_hatchery_job_runs(
        &self,
        _hatchery_id: &str,
        _status: Option<&str>,
    ) -> crate::error::Result<Vec<JobRun>> {
        Err(OverseerError::Internal("not yet implemented".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get_test_db() -> Option<PostgresDatabase> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        if !url.starts_with("postgres") {
            return None;
        }
        // If the env var IS set, we expect it to work
        Some(
            PostgresDatabase::open(&url)
                .await
                .expect("TEST_DATABASE_URL is set but database connection failed"),
        )
    }

    #[tokio::test]
    async fn test_postgres_artifact_crud() {
        let Some(db) = get_test_db().await else {
            return;
        };

        let artifact_id = Uuid::new_v4().to_string();
        let artifact = db
            .insert_artifact(&artifact_id, "report.pdf", "application/pdf", 1024, None)
            .await
            .expect("insert succeeds");

        assert_eq!(artifact.id, artifact_id);
        assert_eq!(artifact.name, "report.pdf");

        let fetched = db
            .get_artifact(&artifact_id)
            .await
            .expect("get succeeds")
            .expect("artifact exists");
        assert_eq!(fetched.id, artifact.id);

        let all = db.list_artifacts(None).await.expect("list succeeds");
        assert!(!all.is_empty());
    }

    #[tokio::test]
    async fn test_postgres_job_definition_crud() {
        let Some(db) = get_test_db().await else {
            return;
        };

        let def = db
            .create_job_definition(
                "pg-test-job",
                "A test job",
                serde_json::json!({"key": "val"}),
            )
            .await
            .expect("create succeeds");

        assert_eq!(def.name, "pg-test-job");

        let fetched = db
            .get_job_definition(&def.id)
            .await
            .expect("get succeeds")
            .expect("definition exists");
        assert_eq!(fetched.id, def.id);
    }

    #[tokio::test]
    async fn test_postgres_decision_crud() {
        let Some(db) = get_test_db().await else {
            return;
        };

        let tags = vec!["test".to_string()];
        let dec = db
            .log_decision("agent-pg", "ctx", "dec", "reason", &tags, None)
            .await
            .expect("log succeeds");

        assert_eq!(dec.agent, "agent-pg");

        let fetched = db
            .get_decision(&dec.id)
            .await
            .expect("get succeeds")
            .expect("decision exists");
        assert_eq!(fetched.id, dec.id);
    }
}
