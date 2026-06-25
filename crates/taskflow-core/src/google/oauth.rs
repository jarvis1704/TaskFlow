use oauth2::{
    AuthUrl, ClientId, ClientSecret, PkceCodeChallenge, RedirectUrl, Scope, TokenUrl,
};
use oauth2::basic::BasicClient;
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use reqwest::Url;

#[derive(Deserialize, Debug, Clone)]
pub struct Credentials {
    pub installed: InstalledDetails,
}

#[derive(Deserialize, Debug, Clone)]
pub struct InstalledDetails {
    pub client_id: String,
    pub client_secret: String,
    pub auth_uri: String,
    pub token_uri: String,
    pub redirect_uris: Vec<String>,
}

fn load_from_dir(dir: &std::path::Path) -> Option<Result<Credentials, String>> {
    let oauth_client = dir.join("oauth_client.json");
    if oauth_client.exists() {
        let content = match std::fs::read_to_string(&oauth_client) {
            Ok(c) => c,
            Err(e) => return Some(Err(format!("Failed to read oauth_client.json: {}", e))),
        };
        return Some(serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse oauth_client.json: {}", e)));
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            if filename.starts_with("client_secret_") && filename.ends_with(".json") {
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => return Some(Err(format!("Failed to read {}: {}", filename, e))),
                };
                return Some(serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse {}: {}", filename, e)));
            }
        }
    }

    None
}

pub fn load_credentials() -> Result<Credentials, String> {
    // 1. Current working directory
    if let Some(res) = load_from_dir(std::path::Path::new(".")) {
        return res;
    }

    // Get standard project directories
    if let Some(proj_dirs) = directories::ProjectDirs::from("org", "taskflow", "taskflow") {
        // 2. Config directory
        if let Some(res) = load_from_dir(proj_dirs.config_dir()) {
            return res;
        }
        // 3. Data directory
        if let Some(res) = load_from_dir(proj_dirs.data_dir()) {
            return res;
        }
    }

    Err("OAuth credentials file not found. Please ensure client_secret_*.json or oauth_client.json exists in the current directory or config directory (~/.config/taskflow/).".to_string())
}

pub async fn run_oauth_flow() -> Result<(String, u64, Option<String>), String> {
    let creds = load_credentials()?;
    
    // Start local loopback listener on a random free port
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to bind local port: {}", e))?;
    let port = listener.local_addr().unwrap().port();
    
    let redirect_url = format!("http://localhost:{}", port);
    
    let client = BasicClient::new(
        ClientId::new(creds.installed.client_id.clone()),
    )
    .set_client_secret(ClientSecret::new(creds.installed.client_secret.clone()))
    .set_auth_uri(AuthUrl::new(creds.installed.auth_uri.clone()).map_err(|e| e.to_string())?)
    .set_token_uri(TokenUrl::new(creds.installed.token_uri.clone()).map_err(|e| e.to_string())?)
    .set_redirect_uri(RedirectUrl::new(redirect_url.clone()).map_err(|e| e.to_string())?);

    // Generate PKCE code challenge & verifier
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate auth URL
    let (auth_url, _csrf_token) = client
        .authorize_url(oauth2::CsrfToken::new_random)
        .add_scope(Scope::new("https://www.googleapis.com/auth/tasks".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .url();

    println!("Opening browser to login to Google Tasks...");
    println!("URL: {}", auth_url);

    // Open browser
    if let Err(e) = webbrowser::open(auth_url.as_str()) {
        println!("Warning: Could not open web browser automatically: {}. Please copy/paste the URL manually.", e);
    }

    // Await loopback authorization code redirection
    let code = receive_auth_code(listener)?;

    // Exchange the authorization code for tokens
    println!("Exchanging authorization code for tokens...");
    let token_url = &creds.installed.token_uri;
    
    let client_http = reqwest::Client::new();
    let res = client_http
        .post(token_url)
        .form(&[
            ("client_id", &creds.installed.client_id),
            ("client_secret", &creds.installed.client_secret),
            ("code", &code),
            ("code_verifier", pkce_verifier.secret()),
            ("grant_type", &"authorization_code".to_string()),
            ("redirect_uri", &redirect_url),
        ])
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed: {}", err_text));
    }

    let token_resp: super::token::TokenResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    Ok((token_resp.access_token, token_resp.expires_in, token_resp.refresh_token))
}

fn receive_auth_code(listener: TcpListener) -> Result<String, String> {
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|e| format!("Connection failed: {}", e))?;
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)
            .map_err(|e| format!("Failed to read request line: {}", e))?;

        // Format: GET /callback?code=xxx HTTP/1.1
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 || parts[0] != "GET" {
            continue;
        }

        let uri = parts[1];
        let full_url = format!("http://localhost{}", uri);
        let parsed_url = Url::parse(&full_url)
            .map_err(|e| format!("Failed to parse redirect URL: {}", e))?;
        
        let code = parsed_url.query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned());

        if let Some(auth_code) = code {
            // Respond with a simple success page
            let response = "HTTP/1.1 200 OK\r\n\
                            Content-Type: text/html; charset=utf-8\r\n\
                            Connection: close\r\n\r\n\
                            <!DOCTYPE html>\
                            <html>\
                            <head><title>TaskFlow Auth Success</title></head>\
                            <body style=\"font-family: sans-serif; text-align: center; padding-top: 50px; background-color: #15161B; color: #E7E8EC;\">\
                                <h1 style=\"color: #5FD9A4;\">Authentication Successful!</h1>\
                                <p>You have successfully logged in to TaskFlow.</p>\
                                <p>You can close this tab and return to the application.</p>\
                            </body>\
                            </html>";
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            return Ok(auth_code);
        } else {
            let response = "HTTP/1.1 400 Bad Request\r\n\
                            Content-Type: text/plain\r\n\
                            Connection: close\r\n\r\n\
                            Authorization code not found in request.";
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            return Err("Authorization code not found".to_string());
        }
    }

    Err("Listener closed without receiving authorization code".to_string())
}
