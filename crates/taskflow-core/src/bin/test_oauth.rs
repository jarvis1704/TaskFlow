use taskflow_core::google::oauth::run_oauth_flow;
use taskflow_core::google::token::TokenManager;
use taskflow_core::google::tasks_api::GoogleTasksClient;

#[tokio::main]
async fn main() {
    println!("=== TaskFlow OAuth Flow End-to-End Test ===");
    println!("This test will open your browser and prompt you to authorize TaskFlow.");
    println!("Press Enter to begin the OAuth flow...");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);

    match run_oauth_flow().await {
        Ok((access_token, expires_in, refresh_token)) => {
            println!("\nOAuth flow succeeded!");
            println!("Access Token: {}...", &access_token[..std::cmp::min(15, access_token.len())]);
            println!("Expires In: {} seconds", expires_in);
            if let Some(ref ref_token) = refresh_token {
                println!("Refresh Token: {}...", &ref_token[..std::cmp::min(15, ref_token.len())]);
                println!("Saving refresh token to OS keyring...");
                match TokenManager::save_refresh_token(ref_token) {
                    Ok(_) => println!("Successfully saved refresh token to keyring!"),
                    Err(e) => println!("Error saving to keyring: {}", e),
                }
            } else {
                println!("No refresh token returned (might already be authorized or access_type=offline missing).");
            }

            // Verify using the client
            println!("\nTesting API client list_task_lists...");
            let creds = taskflow_core::google::oauth::load_credentials().unwrap();
            let mut token_manager = TokenManager::new();
            token_manager.set_tokens(access_token, expires_in, refresh_token);
            let mut client = GoogleTasksClient::new(creds, token_manager);

            match client.list_task_lists().await {
                Ok(lists) => {
                    println!("Successfully listed {} task lists:", lists.len());
                    for list in lists {
                        println!(" - {} (ID: {})", list.title, list.id);
                    }
                }
                Err(e) => {
                    println!("Failed to list task lists: {}", e);
                }
            }
        }
        Err(e) => {
            println!("\nOAuth flow failed: {}", e);
        }
    }
}
