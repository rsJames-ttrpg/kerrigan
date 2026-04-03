use base64::Engine as _;
use serde::Serialize;
use serde_json::Value;

use crate::error::Error;
use crate::types::{
    Artifact, Credential, Hatchery, JobDefinition, JobRun, MatchedCredential, Task,
};

#[derive(Debug, Clone)]
pub struct NydusClient {
    pub(crate) base_url: String,
    client: reqwest::Client,
}

impl NydusClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    async fn check_response(resp: reqwest::Response) -> Result<reqwest::Response, Error> {
        if resp.status().is_success() {
            Ok(resp)
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(Error::Api { status, body })
        }
    }

    // --- Jobs: Definitions ---

    pub async fn create_definition(
        &self,
        name: &str,
        description: &str,
        config: Value,
    ) -> Result<JobDefinition, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            description: &'a str,
            config: Value,
        }
        let resp = self
            .client
            .post(format!("{}/api/jobs/definitions", self.base_url))
            .json(&Body {
                name,
                description,
                config,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn get_definition(&self, id: &str) -> Result<JobDefinition, Error> {
        let resp = self
            .client
            .get(format!("{}/api/jobs/definitions/{id}", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn list_definitions(&self) -> Result<Vec<JobDefinition>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/jobs/definitions", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    // --- Jobs: Runs ---

    pub async fn start_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
        config_overrides: Option<Value>,
    ) -> Result<JobRun, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            definition_id: &'a str,
            triggered_by: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            parent_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            config_overrides: Option<Value>,
        }
        let resp = self
            .client
            .post(format!("{}/api/jobs/runs", self.base_url))
            .json(&Body {
                definition_id,
                triggered_by,
                parent_id,
                config_overrides,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn list_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>, Error> {
        let url = match status {
            Some(s) => format!("{}/api/jobs/runs?status={s}", self.base_url),
            None => format!("{}/api/jobs/runs", self.base_url),
        };
        let resp = self.client.get(url).send().await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn update_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<Value>,
        error: Option<&str>,
    ) -> Result<JobRun, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            status: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            result: Option<Value>,
            #[serde(skip_serializing_if = "Option::is_none")]
            error: Option<&'a str>,
        }
        let resp = self
            .client
            .patch(format!("{}/api/jobs/runs/{id}", self.base_url))
            .json(&Body {
                status,
                result,
                error,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn advance_run(&self, id: &str) -> Result<JobRun, Error> {
        let resp = self
            .client
            .post(format!("{}/api/jobs/runs/{id}/advance", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    // --- Tasks ---

    pub async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Task, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            subject: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            run_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            assigned_to: Option<&'a str>,
        }
        let resp = self
            .client
            .post(format!("{}/api/tasks", self.base_url))
            .json(&Body {
                subject,
                run_id,
                assigned_to,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<Task>, Error> {
        let mut params: Vec<String> = Vec::new();
        if let Some(s) = status {
            params.push(format!("status={s}"));
        }
        if let Some(a) = assigned_to {
            params.push(format!("assigned_to={a}"));
        }
        if let Some(r) = run_id {
            params.push(format!("run_id={r}"));
        }
        let url = if params.is_empty() {
            format!("{}/api/tasks", self.base_url)
        } else {
            format!("{}/api/tasks?{}", self.base_url, params.join("&"))
        };
        let resp = self.client.get(url).send().await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<Value>,
    ) -> Result<Task, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            status: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            assigned_to: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            output: Option<Value>,
        }
        let resp = self
            .client
            .patch(format!("{}/api/tasks/{id}", self.base_url))
            .json(&Body {
                status,
                assigned_to,
                output,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    // --- Hatcheries ---

    pub async fn register_hatchery(
        &self,
        name: &str,
        capabilities: Value,
        max_concurrency: i32,
    ) -> Result<Hatchery, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            capabilities: Value,
            max_concurrency: i32,
        }
        let resp = self
            .client
            .post(format!("{}/api/hatcheries", self.base_url))
            .json(&Body {
                name,
                capabilities,
                max_concurrency,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn heartbeat(
        &self,
        hatchery_id: &str,
        status: &str,
        active_drones: i32,
    ) -> Result<Hatchery, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            status: &'a str,
            active_drones: i32,
        }
        let resp = self
            .client
            .post(format!(
                "{}/api/hatcheries/{hatchery_id}/heartbeat",
                self.base_url
            ))
            .json(&Body {
                status,
                active_drones,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn get_hatchery(&self, id: &str) -> Result<Hatchery, Error> {
        let resp = self
            .client
            .get(format!("{}/api/hatcheries/{id}", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn list_hatcheries(&self, status: Option<&str>) -> Result<Vec<Hatchery>, Error> {
        let url = match status {
            Some(s) => format!("{}/api/hatcheries?status={s}", self.base_url),
            None => format!("{}/api/hatcheries", self.base_url),
        };
        let resp = self.client.get(url).send().await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn deregister_hatchery(&self, id: &str) -> Result<(), Error> {
        let resp = self
            .client
            .delete(format!("{}/api/hatcheries/{id}", self.base_url))
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn list_hatchery_jobs(
        &self,
        hatchery_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<JobRun>, Error> {
        let url = match status {
            Some(s) => {
                format!(
                    "{}/api/hatcheries/{hatchery_id}/jobs?status={s}",
                    self.base_url
                )
            }
            None => format!("{}/api/hatcheries/{hatchery_id}/jobs", self.base_url),
        };
        let resp = self.client.get(url).send().await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn assign_job(&self, hatchery_id: &str, job_run_id: &str) -> Result<JobRun, Error> {
        let resp = self
            .client
            .put(format!(
                "{}/api/hatcheries/{hatchery_id}/jobs/{job_run_id}",
                self.base_url
            ))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn list_pending_runs(&self) -> Result<Vec<JobRun>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/jobs/runs/pending", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    // --- Artifacts ---

    pub async fn store_artifact(
        &self,
        name: &str,
        content_type: &str,
        data: &[u8],
        run_id: Option<&str>,
        artifact_type: Option<&str>,
    ) -> Result<Artifact, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            content_type: &'a str,
            data: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            run_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            artifact_type: Option<&'a str>,
        }
        let data = base64::engine::general_purpose::STANDARD.encode(data);
        let resp = self
            .client
            .post(format!("{}/api/artifacts", self.base_url))
            .json(&Body {
                name,
                content_type,
                data,
                run_id,
                artifact_type,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn get_artifact(&self, id: &str) -> Result<Vec<u8>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/artifacts/{id}", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.bytes().await?.to_vec())
    }

    pub async fn list_artifacts(
        &self,
        run_id: Option<&str>,
        artifact_type: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<Artifact>, Error> {
        let url = format!("{}/api/artifacts", self.base_url);
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(r) = run_id {
            query.push(("run_id", r));
        }
        if let Some(at) = artifact_type {
            query.push(("artifact_type", at));
        }
        if let Some(s) = since {
            query.push(("since", s));
        }
        let resp = self.client.get(url).query(&query).send().await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    // --- Auth ---

    pub async fn submit_auth_code(&self, job_run_id: &str, code: &str) -> Result<(), Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            code: &'a str,
        }
        let resp = self
            .client
            .post(format!("{}/api/jobs/runs/{job_run_id}/auth", self.base_url))
            .json(&Body { code })
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn poll_auth_code(&self, job_run_id: &str) -> Result<Option<String>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/jobs/runs/{job_run_id}/auth", self.base_url))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        #[derive(serde::Deserialize)]
        struct AuthResp {
            code: String,
        }
        let body: AuthResp = Self::check_response(resp).await?.json().await?;
        Ok(Some(body.code))
    }

    // --- Credentials ---

    pub async fn create_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            pattern: &'a str,
            credential_type: &'a str,
            secret: &'a str,
        }
        let resp = self
            .client
            .post(format!("{}/api/credentials", self.base_url))
            .json(&Body {
                pattern,
                credential_type,
                secret,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn list_credentials(&self) -> Result<Vec<Credential>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/credentials", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn delete_credential(&self, id: &str) -> Result<(), Error> {
        let resp = self
            .client
            .delete(format!("{}/api/credentials/{id}", self.base_url))
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn match_credentials(&self, repo_url: &str) -> Result<Vec<MatchedCredential>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/credentials/match", self.base_url))
            .query(&[("repo_url", repo_url)])
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = NydusClient::new("http://localhost:3100");
        assert_eq!(client.base_url, "http://localhost:3100");
    }

    #[test]
    fn test_client_strips_trailing_slash() {
        let client = NydusClient::new("http://localhost:3100/");
        assert_eq!(client.base_url, "http://localhost:3100");
    }
}
