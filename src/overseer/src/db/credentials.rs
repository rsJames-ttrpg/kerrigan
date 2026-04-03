use chrono::NaiveDateTime;
use sea_query::{Expr, Order, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::models::Credential;
use super::tables::Credentials;
use crate::error::{OverseerError, Result};

pub(crate) fn row_to_credential(row: &sqlx::sqlite::SqliteRow) -> Credential {
    Credential {
        id: row.get("id"),
        pattern: row.get("pattern"),
        credential_type: row
            .get::<String, _>("credential_type")
            .parse()
            .expect("CredentialType::from_str is infallible"),
        secret: row.get("secret"),
        created_at: row.get::<NaiveDateTime, _>("created_at").and_utc(),
        updated_at: row.get::<NaiveDateTime, _>("updated_at").and_utc(),
    }
}

pub(crate) async fn create_credential(
    pool: &SqlitePool,
    pattern: &str,
    credential_type: &str,
    secret: &str,
) -> Result<Credential> {
    let id = Uuid::new_v4().to_string();
    let (sql, values) = Query::insert()
        .into_table(Credentials::Table)
        .columns([
            Credentials::Id,
            Credentials::Pattern,
            Credentials::CredentialType,
            Credentials::Secret,
        ])
        .values_panic([
            id.clone().into(),
            pattern.into(),
            credential_type.into(),
            secret.into(),
        ])
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    get_credential(pool, &id)
        .await?
        .ok_or_else(|| OverseerError::Internal("credential not found after insert".into()))
}

pub(crate) async fn get_credential(pool: &SqlitePool, id: &str) -> Result<Option<Credential>> {
    let (sql, values) = Query::select()
        .column(sea_query::Asterisk)
        .from(Credentials::Table)
        .and_where(Expr::col(Credentials::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_credential))
}

pub(crate) async fn delete_credential(pool: &SqlitePool, id: &str) -> Result<()> {
    let (sql, values) = Query::delete()
        .from_table(Credentials::Table)
        .and_where(Expr::col(Credentials::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(())
}

pub(crate) async fn list_credentials(pool: &SqlitePool) -> Result<Vec<Credential>> {
    let (sql, values) = Query::select()
        .column(sea_query::Asterisk)
        .from(Credentials::Table)
        .order_by(Credentials::CreatedAt, Order::Desc)
        .build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(row_to_credential).collect())
}

pub(crate) async fn upsert_credential(
    pool: &SqlitePool,
    pattern: &str,
    credential_type: &str,
    secret: &str,
) -> Result<Credential> {
    // Check if exists
    let (sql, values) = Query::select()
        .column(sea_query::Asterisk)
        .from(Credentials::Table)
        .and_where(Expr::col(Credentials::Pattern).eq(pattern))
        .and_where(Expr::col(Credentials::CredentialType).eq(credential_type))
        .build_sqlx(SqliteQueryBuilder);

    let existing = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    if let Some(row) = existing {
        let id: String = row.get("id");
        let (sql, values) = Query::update()
            .table(Credentials::Table)
            .value(Credentials::Secret, secret)
            .value(Credentials::UpdatedAt, sea_query::Expr::current_timestamp())
            .and_where(Expr::col(Credentials::Id).eq(&id))
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(pool)
            .await
            .map_err(OverseerError::Storage)?;

        get_credential(pool, &id)
            .await?
            .ok_or_else(|| OverseerError::Internal("credential not found after upsert".into()))
    } else {
        create_credential(pool, pattern, credential_type, secret).await
    }
}

pub(crate) async fn match_credentials(
    pool: &SqlitePool,
    repo_url: &str,
) -> Result<Vec<Credential>> {
    use nydus::normalize::{normalize_repo_url, pattern_matches};

    let normalized = normalize_repo_url(repo_url);
    let all = list_credentials(pool).await?;

    // Group by credential_type, keep best (highest specificity) match per type
    let mut best: std::collections::HashMap<String, (usize, Credential)> =
        std::collections::HashMap::new();

    for cred in all {
        let normalized_pattern = normalize_repo_url(&cred.pattern);
        if let Some(score) = pattern_matches(&normalized, &normalized_pattern) {
            let type_key = cred.credential_type.to_string();
            match best.entry(type_key) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert((score, cred));
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if score > e.get().0 {
                        e.insert((score, cred));
                    }
                }
            }
        }
    }

    Ok(best.into_values().map(|(_, cred)| cred).collect())
}

#[cfg(test)]
mod tests {
    use crate::db::SqliteDatabase;

    #[tokio::test]
    async fn test_credential_crud() {
        let db = SqliteDatabase::open_in_memory_named("cred_crud_test")
            .await
            .expect("db opens");

        let cred = super::create_credential(
            &db.pool,
            "github.com/rsJames-ttrpg/*",
            "github_pat",
            "ghp_test123",
        )
        .await
        .expect("create");
        assert_eq!(cred.pattern, "github.com/rsJames-ttrpg/*");
        assert_eq!(cred.secret, "ghp_test123");

        let fetched = super::get_credential(&db.pool, &cred.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(fetched.id, cred.id);

        let all = super::list_credentials(&db.pool).await.expect("list");
        assert_eq!(all.len(), 1);

        super::delete_credential(&db.pool, &cred.id)
            .await
            .expect("delete");

        let gone = super::get_credential(&db.pool, &cred.id)
            .await
            .expect("get after delete");
        assert!(gone.is_none());
    }

    #[tokio::test]
    async fn test_credential_upsert() {
        let db = SqliteDatabase::open_in_memory_named("cred_upsert_test")
            .await
            .expect("db opens");

        let cred1 =
            super::upsert_credential(&db.pool, "github.com/org/*", "github_pat", "old_secret")
                .await
                .expect("first upsert");

        let cred2 =
            super::upsert_credential(&db.pool, "github.com/org/*", "github_pat", "new_secret")
                .await
                .expect("second upsert");

        assert_eq!(cred1.id, cred2.id);
        assert_eq!(cred2.secret, "new_secret");

        let all = super::list_credentials(&db.pool).await.expect("list");
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_credential_match_best_per_type() {
        let db = SqliteDatabase::open_in_memory_named("cred_match_test")
            .await
            .expect("db opens");

        // Broad wildcard
        super::create_credential(
            &db.pool,
            "github.com/rsJames-ttrpg/*",
            "github_pat",
            "broad_pat",
        )
        .await
        .expect("create broad");

        // Exact match (more specific)
        super::create_credential(
            &db.pool,
            "github.com/rsJames-ttrpg/kerrigan",
            "github_pat",
            "exact_pat",
        )
        .await
        .expect("create exact");

        let matches =
            super::match_credentials(&db.pool, "git@github.com:rsJames-ttrpg/kerrigan.git")
                .await
                .expect("match");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].secret, "exact_pat");
    }

    #[tokio::test]
    async fn test_credential_match_multiple_types() {
        let db = SqliteDatabase::open_in_memory_named("cred_match_multi_test")
            .await
            .expect("db opens");

        super::create_credential(&db.pool, "github.com/org/*", "github_pat", "my_pat")
            .await
            .expect("create pat");

        // Simulate a second credential type by inserting directly
        // (CredentialType only has GithubPat for now, but the DB stores strings)
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO credentials (id, pattern, credential_type, secret) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind("github.com/org/*")
        .bind("deployment_key")
        .bind("my_deploy_key")
        .execute(&db.pool)
        .await
        .expect("insert deployment_key");

        let matches = super::match_credentials(&db.pool, "https://github.com/org/repo.git")
            .await
            .expect("match");

        assert_eq!(matches.len(), 2);
    }
}
