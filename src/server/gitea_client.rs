#![allow(dead_code)] // WIP scaffolding not yet wired
//! Gitea HTTP API Client
//!
//! Replaces libgit2 for server operations - all git ops go through Gitea REST API.
//! This is Send-safe and follows the external-memfs pattern.

use anyhow::{anyhow, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct GiteaClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
    username: tokio::sync::OnceCell<String>,
}

#[derive(Debug, Deserialize)]
pub struct GiteaRepo {
    pub id: i64,
    pub name: String,
    pub clone_url: String,
}

#[derive(Debug, Deserialize)]
struct GiteaFileResponse {
    #[serde(rename = "type")]
    file_type: String,
    name: String,
    path: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GiteaBlobResponse {
    content: String,
    sha: String,
}

#[derive(Debug, Serialize)]
struct CreateRepoRequest {
    name: String,
    description: String,
    private: bool,
}

#[derive(Debug, Serialize)]
struct CreateFileRequest {
    content: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct UpdateFileRequest {
    content: String,
    message: String,
    sha: String,
}

impl GiteaClient {
    pub fn new(base_url: String, token: String) -> Self {
        let client = reqwest::Client::new();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            client,
            username: tokio::sync::OnceCell::new(),
        }
    }

    fn auth_header(&self) -> String {
        format!("token {}", self.token)
    }

    /// Get username (cached)
    async fn get_username(&self) -> Result<&str> {
        self.username
            .get_or_try_init(|| async {
                let url = format!("{}/api/v1/user", self.base_url);
                let resp = self
                    .client
                    .get(&url)
                    .header("Authorization", self.auth_header())
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    return Err(anyhow!("Failed to get user: {}", resp.status()));
                }

                let user: serde_json::Value = resp.json().await?;
                let username = user
                    .get("login")
                    .and_then(|l| l.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "souveraine".to_string());

                Ok::<String, anyhow::Error>(username)
            })
            .await
            .map(|s| s.as_str())
    }

    fn repo_name(&self, agent_id: &str) -> String {
        format!("agent-{}", agent_id)
    }

    /// Create a repository for an agent
    pub async fn create_repo(&self, agent_id: &str) -> Result<GiteaRepo> {
        let url = format!("{}/api/v1/user/repos", self.base_url);
        let body = CreateRepoRequest {
            name: self.repo_name(agent_id),
            description: format!("Souveraine agent {} memory repository", agent_id),
            private: true,
        };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to create repo: {}", resp.status()));
        }

        Ok(resp.json().await?)
    }

    /// Check if repository exists
    pub async fn repo_exists(&self, agent_id: &str) -> Result<bool> {
        let owner = self.get_username().await?;
        let url = format!(
            "{}/api/v1/repos/{}/{}",
            self.base_url,
            owner,
            self.repo_name(agent_id)
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        Ok(resp.status().is_success())
    }

    /// Get raw file content
    pub async fn get_file(&self, agent_id: &str, path: &str) -> Result<Option<String>> {
        let owner = self.get_username().await?;
        let repo = self.repo_name(agent_id);
        let url = format!(
            "{}/api/v1/repos/{}/{}/raw/HEAD/{}",
            self.base_url, owner, repo, path
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if resp.status().as_u16() == 404 {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to get file: {}", resp.status()));
        }

        Ok(Some(resp.text().await?))
    }

    /// Get file SHA (needed for updates)
    async fn get_file_sha(&self, agent_id: &str, path: &str) -> Result<Option<String>> {
        let owner = self.get_username().await?;
        let repo = self.repo_name(agent_id);
        let url = format!(
            "{}/api/v1/repos/{}/{}/contents/{}",
            self.base_url, owner, repo, path
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if resp.status().as_u16() == 404 {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to get file info: {}", resp.status()));
        }

        let info: serde_json::Value = resp.json().await?;
        Ok(info
            .get("sha")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()))
    }

    /// Create or update file
    pub async fn put_file(
        &self,
        agent_id: &str,
        path: &str,
        content: &str,
        message: &str,
    ) -> Result<()> {
        let owner = self.get_username().await?;
        let repo = self.repo_name(agent_id);
        let url = format!(
            "{}/api/v1/repos/{}/{}/contents/{}",
            self.base_url, owner, repo, path
        );

        // Check if file exists first
        let existing_sha = self.get_file_sha(agent_id, path).await?;

        let encoded_content = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());

        let resp = if let Some(sha) = existing_sha {
            // Update
            let body = UpdateFileRequest {
                content: encoded_content,
                message: message.to_string(),
                sha,
            };
            self.client
                .put(&url)
                .header("Authorization", self.auth_header())
                .json(&body)
                .send()
                .await?
        } else {
            // Create
            let body = CreateFileRequest {
                content: encoded_content,
                message: message.to_string(),
            };
            self.client
                .post(&url)
                .header("Authorization", self.auth_header())
                .json(&body)
                .send()
                .await?
        };

        if !resp.status().is_success() {
            return Err(anyhow!(
                "Failed to put file: {}",
                resp.text().await.unwrap_or_default()
            ));
        }

        Ok(())
    }

    /// List files in directory
    pub async fn list_files(&self, agent_id: &str, path: &str) -> Result<Vec<String>> {
        let owner = self.get_username().await?;
        let repo = self.repo_name(agent_id);
        let url = format!(
            "{}/api/v1/repos/{}/{}/contents/{}",
            self.base_url, owner, repo, path
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if resp.status().as_u16() == 404 {
            return Ok(vec![]);
        }

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to list files: {}", resp.status()));
        }

        let files: Vec<serde_json::Value> = resp.json().await?;
        Ok(files
            .iter()
            .filter(|f| f.get("type").and_then(|t| t.as_str()) == Some("file"))
            .filter_map(|f| {
                f.get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect())
    }

    /// Get commit history
    pub async fn get_commits(&self, agent_id: &str, limit: usize) -> Result<Vec<(String, String)>> {
        let owner = self.get_username().await?;
        let repo = self.repo_name(agent_id);
        let url = format!(
            "{}/api/v1/repos/{}/{}/commits?limit={}",
            self.base_url, owner, repo, limit
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to get commits: {}", resp.status()));
        }

        let commits: Vec<serde_json::Value> = resp.json().await?;
        Ok(commits
            .iter()
            .filter_map(|c| {
                let sha = c.get("sha")?.as_str()?.to_string();
                let msg = c.get("commit")?.get("message")?.as_str()?.to_string();
                Some((sha, msg))
            })
            .collect())
    }
}
