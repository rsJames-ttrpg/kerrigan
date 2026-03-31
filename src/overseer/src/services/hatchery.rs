use std::sync::Arc;

use crate::db::Database;
use crate::db::models::{Hatchery, JobRun};
use crate::error::Result;

pub struct HatcheryService {
    db: Arc<dyn Database>,
}

impl HatcheryService {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    pub async fn register(
        &self,
        name: &str,
        capabilities: serde_json::Value,
        max_concurrency: i32,
    ) -> Result<Hatchery> {
        self.db
            .register_hatchery(name, capabilities, max_concurrency)
            .await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Hatchery>> {
        self.db.get_hatchery(id).await
    }

    pub async fn get_by_name(&self, name: &str) -> Result<Option<Hatchery>> {
        self.db.get_hatchery_by_name(name).await
    }

    pub async fn heartbeat(&self, id: &str, status: &str, active_drones: i32) -> Result<Hatchery> {
        self.db.heartbeat_hatchery(id, status, active_drones).await
    }

    pub async fn list(&self, status: Option<&str>) -> Result<Vec<Hatchery>> {
        self.db.list_hatcheries(status).await
    }

    pub async fn deregister(&self, id: &str) -> Result<()> {
        self.db.deregister_hatchery(id).await
    }

    pub async fn assign_job(&self, job_run_id: &str, hatchery_id: &str) -> Result<JobRun> {
        self.db
            .assign_job_to_hatchery(job_run_id, hatchery_id)
            .await
    }

    pub async fn list_job_runs(
        &self,
        hatchery_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<JobRun>> {
        self.db.list_hatchery_job_runs(hatchery_id, status).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDatabase;

    #[tokio::test]
    async fn test_hatchery_service_register_and_heartbeat() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_hatchery_test_register")
            .await
            .expect("db opens");
        let svc = HatcheryService::new(Arc::new(sqlite_db));

        let h = svc
            .register("test-hatchery", serde_json::json!({"arch": "x86_64"}), 4)
            .await
            .expect("register");
        assert_eq!(h.name, "test-hatchery");
        assert_eq!(h.max_concurrency, 4);

        let fetched = svc.get(&h.id).await.expect("get").expect("exists");
        assert_eq!(fetched.id, h.id);

        let by_name = svc
            .get_by_name("test-hatchery")
            .await
            .expect("get_by_name")
            .expect("exists");
        assert_eq!(by_name.id, h.id);

        let updated = svc
            .heartbeat(&h.id, "degraded", 2)
            .await
            .expect("heartbeat");
        assert_eq!(updated.status, crate::db::models::HatcheryStatus::Degraded);
        assert_eq!(updated.active_drones, 2);

        let all = svc.list(None).await.expect("list");
        assert_eq!(all.len(), 1);

        let degraded = svc.list(Some("degraded")).await.expect("list degraded");
        assert_eq!(degraded.len(), 1);

        let online = svc.list(Some("online")).await.expect("list online");
        assert_eq!(online.len(), 0);
    }

    #[tokio::test]
    async fn test_hatchery_service_deregister() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_hatchery_test_deregister")
            .await
            .expect("db opens");
        let svc = HatcheryService::new(Arc::new(sqlite_db));

        let h = svc
            .register("hatchery-to-delete", serde_json::json!({}), 1)
            .await
            .expect("register");

        svc.deregister(&h.id).await.expect("deregister");

        let fetched = svc.get(&h.id).await.expect("get after deregister");
        assert!(fetched.is_none());
    }
}
