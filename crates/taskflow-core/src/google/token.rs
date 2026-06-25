use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use std::path::PathBuf;
use directories::ProjectDirs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TokenManager {
    refresh_token: Option<String>,
    access_token: Option<String>,
    expires_at: Option<Instant>,
}

fn get_token_file_path() -> Option<PathBuf> {
    ProjectDirs::from("org", "taskflow", "taskflow")
        .map(|proj_dirs| proj_dirs.config_dir().join("refresh_token"))
}

impl TokenManager {
    pub fn new() -> Self {
        let refresh_token = Self::load_refresh_token().ok().flatten();
        Self {
            refresh_token,
            access_token: None,
            expires_at: None,
        }
    }

    /// Load the refresh token from the OS keyring (or fallback file)
    pub fn load_refresh_token() -> Result<Option<String>, String> {
        let entry_res = Entry::new("taskflow", "google-tasks");
        
        let keyring_result = match entry_res {
            Ok(entry) => match entry.get_password() {
                Ok(pwd) => Some(pwd),
                _ => None,
            },
            _ => None,
        };

        if let Some(pwd) = keyring_result {
            return Ok(Some(pwd));
        }

        // Fallback to local file
        if let Some(path) = get_token_file_path() {
            if path.exists() {
                if let Ok(token) = std::fs::read_to_string(&path) {
                    return Ok(Some(token.trim().to_string()));
                }
            }
        }

        Ok(None)
    }

    /// Save the refresh token to the OS keyring and fallback file
    pub fn save_refresh_token(token: &str) -> Result<(), String> {
        // Save to fallback file first for reliability
        if let Some(path) = get_token_file_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&path, token)
                .map_err(|e| format!("Failed to write fallback token file: {}", e))?;
            
            // Set permissions to owner read/write only (0600) on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = std::fs::metadata(&path) {
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(&path, perms);
                }
            }
        }

        // Save to keyring if possible
        if let Ok(entry) = Entry::new("taskflow", "google-tasks") {
            let _ = entry.set_password(token);
        }

        Ok(())
    }

    /// Delete the refresh token from the OS keyring and fallback file
    pub fn delete_refresh_token() -> Result<(), String> {
        // Delete from fallback file
        if let Some(path) = get_token_file_path() {
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }

        // Delete from keyring if possible
        if let Ok(entry) = Entry::new("taskflow", "google-tasks") {
            let _ = entry.delete_credential();
        }

        Ok(())
    }

    pub fn set_tokens(&mut self, access_token: String, expires_in: u64, refresh_token: Option<String>) {
        self.access_token = Some(access_token);
        self.expires_at = Some(Instant::now() + Duration::from_secs(expires_in));
        if let Some(ref_token) = refresh_token {
            if let Err(e) = Self::save_refresh_token(&ref_token) {
                eprintln!("Warning: Failed to save refresh token to keyring: {}", e);
            }
            self.refresh_token = Some(ref_token);
        }
    }

    pub fn get_access_token(&self) -> Option<String> {
        if self.is_access_token_valid() {
            self.access_token.clone()
        } else {
            None
        }
    }

    pub fn has_refresh_token(&self) -> bool {
        self.refresh_token.is_some()
    }

    pub fn refresh_token(&self) -> Option<String> {
        self.refresh_token.clone()
    }

    pub fn clear(&mut self) -> Result<(), String> {
        Self::delete_refresh_token()?;
        self.refresh_token = None;
        self.access_token = None;
        self.expires_at = None;
        Ok(())
    }

    pub async fn ensure_access_token(&mut self, creds: &super::oauth::Credentials) -> Result<String, String> {
        if let Some(token) = self.get_access_token() {
            return Ok(token);
        }

        let refresh_token = self.refresh_token.as_ref()
            .ok_or_else(|| "No refresh token available. User must log in first.".to_string())?;

        println!("Access token expired or missing. Refreshing...");
        let client = reqwest::Client::new();
        let res = client
            .post(&creds.installed.token_uri)
            .form(&[
                ("client_id", &creds.installed.client_id),
                ("client_secret", &creds.installed.client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", &"refresh_token".to_string()),
            ])
            .send()
            .await
            .map_err(|e| format!("Refresh request failed: {}", e))?;

        if !res.status().is_success() {
            let err_text = res.text().await.unwrap_or_default();
            return Err(format!("Failed to refresh token: {}", err_text));
        }

        let token_resp: TokenResponse = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

        self.set_tokens(
            token_resp.access_token.clone(),
            token_resp.expires_in,
            token_resp.refresh_token.clone(), // Google might not return a new refresh token, which is fine
        );

        Ok(token_resp.access_token)
    }

    fn is_access_token_valid(&self) -> bool {
        if let (Some(_), Some(expiry)) = (&self.access_token, self.expires_at) {
            // Buffer of 60 seconds before expiration
            Instant::now() + Duration::from_secs(60) < expiry
        } else {
            false
        }
    }
}

