use sqlx::SqlitePool;

pub struct ArtifactService {
    #[allow(dead_code)]
    pool: SqlitePool,
}

impl ArtifactService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
