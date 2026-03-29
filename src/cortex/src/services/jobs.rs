use sqlx::SqlitePool;

pub struct JobService {
    #[allow(dead_code)]
    pool: SqlitePool,
}

impl JobService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
