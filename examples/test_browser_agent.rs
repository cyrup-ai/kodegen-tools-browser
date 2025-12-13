//! Minimal test for browser_agent timeout issue

use anyhow::{Context, Result};
use serde_json::json;
use tracing::info;
use kodegen_config::BROWSER_AGENT;

mod common;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt().with_env_filter("info").init();

    info!("🔬 Testing browser_agent timeout issue\n");

    // Connect to local browser HTTP server
    let (conn, mut server) = common::connect_to_local_http_server().await?;

    // Wrap client with logging
    let workspace_root = common::find_workspace_root()
        .context("Failed to find workspace root")?;
    let log_path = workspace_root.join("tmp/mcp-client/browser_agent_test.log");
    let client = common::LoggingClient::new(conn.client(), log_path)
        .await
        .context("Failed to create logging client")?;

    // Run test
    let result = test_browser_agent(&client).await;

    // Always close connection
    conn.close().await?;
    server.shutdown().await?;

    result
}

async fn test_browser_agent(client: &common::LoggingClient) -> Result<()> {
    info!("Testing browser_agent with current (broken) pattern...\n");
    
    info!("🔟  browser_agent PROMPT → Compare axum vs actix-web");
    info!("   ⚠️  This should timeout after 30 seconds due to MCP client timeout\n");

    let start = std::time::Instant::now();
    
    let result = client
        .call_tool(
            BROWSER_AGENT,
            json!({
                "action": "PROMPT",
                "task": "Compare axum vs actix-web crates on crates.io - find downloads, latest version, and key features for each",
                "start_url": "https://crates.io",
                "max_steps": 10,
                "temperature": 0.3
                // ❌ Missing: "await_completion_ms": 0
                // This will use default 600000ms (10 minutes)
                // But MCP client will kill it after 30 seconds
            }),
        )
        .await;

    let elapsed = start.elapsed();
    
    match result {
        Ok(_) => {
            info!("   ✅ Completed in {:.2}s", elapsed.as_secs_f64());
            Ok(())
        }
        Err(e) => {
            info!("   ❌ Failed after {:.2}s", elapsed.as_secs_f64());
            info!("   Error: {}", e);
            
            if elapsed.as_secs() >= 29 && elapsed.as_secs() <= 31 {
                info!("\n   ⚠️  CONFIRMED: 30-second timeout issue!");
                info!("   The error occurred at ~30 seconds, confirming the MCP client timeout.");
                info!("\n   Solution: Use await_completion_ms: 0 for fire-and-forget pattern.");
            }
            
            Err(e.into())
        }
    }
}
