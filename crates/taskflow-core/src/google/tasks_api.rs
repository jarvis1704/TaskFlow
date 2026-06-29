use crate::google::oauth::Credentials;
use crate::google::token::TokenManager;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTaskList {
    pub id: String,
    pub title: String,
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTaskListResponse {
    pub items: Option<Vec<GoogleTaskList>>,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTask {
    pub id: Option<String>,
    pub title: Option<String>,
    pub notes: Option<String>,
    pub status: Option<String>,   // "needsAction" | "completed"
    pub due: Option<String>,      // YYYY-MM-DDT00:00:00.000Z
    pub completed: Option<String>,// RFC3339
    pub updated: Option<String>,  // RFC3339
    pub parent: Option<String>,
    pub position: Option<String>,
    pub deleted: Option<bool>,
    pub hidden: Option<bool>,
    pub starred: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTasksResponse {
    pub items: Option<Vec<GoogleTask>>,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

pub struct GoogleTasksClient {
    creds: Credentials,
    token_manager: TokenManager,
    client: reqwest::Client,
}

impl GoogleTasksClient {
    pub fn new(creds: Credentials, token_manager: TokenManager) -> Self {
        Self {
            creds,
            token_manager,
            client: reqwest::Client::new(),
        }
    }

    pub fn token_manager(&self) -> &TokenManager {
        &self.token_manager
    }

    pub fn token_manager_mut(&mut self) -> &mut TokenManager {
        &mut self.token_manager
    }

    /// Perform HTTP request with access token and exponential backoff on 429/5xx
    async fn request(
        &mut self,
        method: reqwest::Method,
        url: &str,
        query: &[(&str, &str)],
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, String> {
        let mut retries = 3;
        let mut delay = Duration::from_secs(1);

        loop {
            // Ensure access token is valid
            let access_token = self.token_manager.ensure_access_token(&self.creds).await?;

            let mut req = self.client.request(method.clone(), url)
                .bearer_auth(access_token)
                .query(query);

            if let Some(ref b) = body {
                req = req.json(b);
            }

            let res = req.send().await
                .map_err(|e| format!("Network request failed: {}", e))?;

            let status = res.status();
            if status.is_success() {
                return Ok(res);
            }

            // Retry on rate limit (429) or server error (5xx)
            if (status.as_u16() == 429 || status.is_server_error()) && retries > 0 {
                warn!("Google Tasks API returned error {}. Retrying in {:?}...", status, delay);
                tokio::time::sleep(delay).await;
                retries -= 1;
                delay *= 2;
            } else {
                let err_text = res.text().await.unwrap_or_default();
                return Err(format!("Google Tasks API error ({}): {}", status, err_text));
            }
        }
    }

    /// List all task lists for the authenticated user
    pub async fn list_task_lists(&mut self) -> Result<Vec<GoogleTaskList>, String> {
        let url = "https://tasks.googleapis.com/tasks/v1/users/@me/lists";
        let mut lists = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut query = Vec::new();
            if let Some(ref token) = page_token {
                query.push(("pageToken", token.as_str()));
            }

            let res = self.request(reqwest::Method::GET, url, &query, None).await?;
            let resp: GoogleTaskListResponse = res.json().await
                .map_err(|e| format!("Failed to parse task lists response: {}", e))?;

            if let Some(items) = resp.items {
                lists.extend(items);
            }

            if resp.next_page_token.is_none() {
                break;
            }
            page_token = resp.next_page_token;
        }

        Ok(lists)
    }

    /// Create a new task list
    pub async fn create_task_list(&mut self, title: &str) -> Result<GoogleTaskList, String> {
        let url = "https://tasks.googleapis.com/tasks/v1/users/@me/lists";
        let body = serde_json::json!({ "title": title });

        let res = self.request(reqwest::Method::POST, url, &[], Some(body)).await?;
        let list: GoogleTaskList = res.json().await
            .map_err(|e| format!("Failed to parse created task list: {}", e))?;
        
        Ok(list)
    }

    /// Delete a task list
    pub async fn delete_task_list(&mut self, list_id: &str) -> Result<(), String> {
        let url = format!("https://tasks.googleapis.com/tasks/v1/users/@me/lists/{}", list_id);
        let _ = self.request(reqwest::Method::DELETE, &url, &[], None).await?;
        Ok(())
    }

    /// List tasks in a task list (with pagination)
    pub async fn list_tasks(
        &mut self,
        list_id: &str,
        updated_min: Option<chrono::DateTime<chrono::Utc>>,
        show_completed: bool,
        show_hidden: bool,
    ) -> Result<Vec<GoogleTask>, String> {
        let url = format!("https://tasks.googleapis.com/tasks/v1/lists/{}/tasks", list_id);
        let mut tasks = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut query = vec![
                ("showCompleted", if show_completed { "true" } else { "false" }),
                ("showHidden", if show_hidden { "true" } else { "false" }),
                ("maxResults", "100"),
            ];

            let updated_min_str;
            if let Some(dt) = updated_min {
                updated_min_str = dt.to_rfc3339();
                query.push(("updatedMin", &updated_min_str));
            }

            if let Some(ref token) = page_token {
                query.push(("pageToken", token.as_str()));
            }

            let res = self.request(reqwest::Method::GET, &url, &query, None).await?;
            let resp: GoogleTasksResponse = res.json().await
                .map_err(|e| format!("Failed to parse tasks response: {}", e))?;

            if let Some(items) = resp.items {
                tasks.extend(items);
            }

            if resp.next_page_token.is_none() {
                break;
            }
            page_token = resp.next_page_token;
        }

        Ok(tasks)
    }

    /// Insert a task into a task list (supporting structural parent and previous positioning)
    pub async fn insert_task(
        &mut self,
        list_id: &str,
        task: &GoogleTask,
        parent: Option<&str>,
        previous: Option<&str>,
    ) -> Result<GoogleTask, String> {
        let url = format!("https://tasks.googleapis.com/tasks/v1/lists/{}/tasks", list_id);
        let body = serde_json::to_value(task)
            .map_err(|e| format!("Failed to serialize task payload: {}", e))?;

        let mut query = Vec::new();
        if let Some(p) = parent {
            query.push(("parent", p));
        }
        if let Some(prev) = previous {
            query.push(("previous", prev));
        }

        let res = self.request(reqwest::Method::POST, &url, &query, Some(body)).await?;
        let created_task: GoogleTask = res.json().await
            .map_err(|e| format!("Failed to parse inserted task: {}", e))?;
        
        Ok(created_task)
    }

    /// Update/patch a task
    pub async fn update_task(&mut self, list_id: &str, task_id: &str, task: &GoogleTask) -> Result<GoogleTask, String> {
        let url = format!("https://tasks.googleapis.com/tasks/v1/lists/{}/tasks/{}", list_id, task_id);
        let body = serde_json::to_value(task)
            .map_err(|e| format!("Failed to serialize task payload: {}", e))?;

        let res = self.request(reqwest::Method::PATCH, &url, &[], Some(body)).await?;
        let updated_task: GoogleTask = res.json().await
            .map_err(|e| format!("Failed to parse updated task: {}", e))?;
        
        Ok(updated_task)
    }

    /// Delete a task
    pub async fn delete_task(&mut self, list_id: &str, task_id: &str) -> Result<(), String> {
        let url = format!("https://tasks.googleapis.com/tasks/v1/lists/{}/tasks/{}", list_id, task_id);
        let _ = self.request(reqwest::Method::DELETE, &url, &[], None).await?;
        Ok(())
    }

    /// Move a task (reparenting or reordering)
    /// Parent and previous are optional Google-assigned task IDs.
    pub async fn move_task(
        &mut self,
        list_id: &str,
        task_id: &str,
        parent: Option<&str>,
        previous: Option<&str>,
    ) -> Result<GoogleTask, String> {
        let url = format!("https://tasks.googleapis.com/tasks/v1/lists/{}/tasks/{}/move", list_id, task_id);
        let mut query = Vec::new();
        if let Some(p) = parent {
            query.push(("parent", p));
        }
        if let Some(prev) = previous {
            query.push(("previous", prev));
        }

        let res = self.request(reqwest::Method::POST, &url, &query, None).await?;
        let moved_task: GoogleTask = res.json().await
            .map_err(|e| format!("Failed to parse moved task: {}", e))?;
        
        Ok(moved_task)
    }
}
