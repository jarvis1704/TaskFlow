use taskflow_core::db::Database;
use taskflow_core::google::oauth::load_credentials;
use taskflow_core::google::token::TokenManager;
use taskflow_core::google::tasks_api::GoogleTasksClient;
use taskflow_core::sync::engine::run_sync;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Initialize tracing to display sync progress details
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    println!("=== TaskFlow Sync Engine Verification ===");

    // 1. Load credentials
    let creds = match load_credentials() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading credentials: {}", e);
            eprintln!("Please ensure your Google OAuth client secret JSON is in the working directory.");
            return;
        }
    };

    // 2. Load token manager
    let token_manager = TokenManager::new();
    if !token_manager.has_refresh_token() {
        eprintln!("\nError: No refresh token found in OS keyring.");
        eprintln!("Please run the OAuth flow first using: cargo run --bin test_oauth");
        return;
    }

    // 3. Connect to local database
    println!("Connecting to SQLite database (default project directory)...");
    let db = match Database::new() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to resolve database directory: {}", e);
            return;
        }
    };
    println!("Database path: {:?}", db.path());

    // 4. Initialize client
    let mut client = GoogleTasksClient::new(creds, token_manager);

    // 5. Run Sync
    println!("\nRunning bidirectional sync engine...");
    match run_sync(&db, &mut client).await {
        Ok(report) => {
            println!("\n==========================================");
            println!("Sync completed successfully!");
            println!("==========================================");
            println!("Lists Pulled (Created locally): {}", report.lists_pulled);
            println!("Lists Pushed (Created remotely): {}", report.lists_pushed);
            println!("Tasks Pulled:                  {}", report.tasks_pulled);
            println!("Tasks Pushed:                  {}", report.tasks_pushed);
            println!("Tasks Deleted (Locally/Remotely): {}", report.tasks_deleted);
            
            if !report.conflicts_resolved.is_empty() {
                println!("\nConflicts Resolved:");
                for conflict in report.conflicts_resolved {
                    println!(" - {}", conflict);
                }
            }
            println!("==========================================");
        }
        Err(e) => {
            eprintln!("\nSync failed with error: {}", e);
        }
    }
}
