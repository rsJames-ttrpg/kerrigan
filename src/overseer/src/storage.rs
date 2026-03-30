use std::sync::Arc;

use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;

use crate::config::StorageConfig;
use crate::error::{OverseerError, Result};

pub fn create_object_store(config: &StorageConfig) -> Result<Arc<dyn ObjectStore>> {
    let url = &config.artifact_url;
    if let Some(path) = url.strip_prefix("file://") {
        let store = LocalFileSystem::new_with_prefix(path)
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;
        Ok(Arc::new(store))
    } else if url.starts_with("s3://") {
        let mut builder = object_store::aws::AmazonS3Builder::from_env().with_url(url);
        if let Some(s3) = &config.s3 {
            if let Some(region) = &s3.region {
                builder = builder.with_region(region);
            }
            if let Some(endpoint) = &s3.endpoint {
                builder = builder.with_endpoint(endpoint);
            }
            if let Some(key_env) = &s3.access_key_env {
                let key = std::env::var(key_env).map_err(|_| {
                    OverseerError::Validation(format!(
                        "S3 access_key_env '{key_env}' configured but environment variable not set"
                    ))
                })?;
                builder = builder.with_access_key_id(key);
            }
            if let Some(secret_env) = &s3.secret_key_env {
                let secret = std::env::var(secret_env).map_err(|_| {
                    OverseerError::Validation(format!(
                        "S3 secret_key_env '{secret_env}' configured but environment variable not set"
                    ))
                })?;
                builder = builder.with_secret_access_key(secret);
            }
        }
        let store = builder
            .build()
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;
        Ok(Arc::new(store))
    } else {
        Err(OverseerError::Validation(format!(
            "unsupported artifact_url scheme: {url}"
        )))
    }
}

pub fn create_in_memory_store() -> Arc<dyn ObjectStore> {
    Arc::new(InMemory::new())
}
