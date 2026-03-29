use sqlx::SqlitePool;

pub struct DecisionService {
    #[allow(dead_code)]
    pool: SqlitePool,
}

impl DecisionService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
