use std::sync::Arc;

use crate::db::Database;
use crate::db::models::Credential;
use crate::error::Result;

pub struct CredentialService {
    db: Arc<dyn Database>,
}

impl CredentialService {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    pub async fn create_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        self.db
            .create_credential(pattern, credential_type, secret)
            .await
    }

    pub async fn get_credential(&self, id: &str) -> Result<Option<Credential>> {
        self.db.get_credential(id).await
    }

    pub async fn delete_credential(&self, id: &str) -> Result<()> {
        self.db.delete_credential(id).await
    }

    pub async fn list_credentials(&self) -> Result<Vec<Credential>> {
        self.db.list_credentials().await
    }

    pub async fn upsert_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        self.db
            .upsert_credential(pattern, credential_type, secret)
            .await
    }

    pub async fn match_credentials(&self, repo_url: &str) -> Result<Vec<Credential>> {
        self.db.match_credentials(repo_url).await
    }
}
