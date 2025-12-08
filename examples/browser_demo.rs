//! Browser tools comprehensive demonstration
//!
//! Demonstrates all 9 public browser tools using real-world examples:
//! - Workflow 1: docs.rs search (7 tools)
//! - Workflow 2: Web search (1 tool)
//! - Workflow 3: AI research (1 tool)
//! - Workflow 4: Autonomous agent (1 tool)

use anyhow::{Context, Result};
use serde_json::json;
use tracing::info;
use kodegen_config::{BROWSER_AGENT, BROWSER_CLICK, BROWSER_EXTRACT_TEXT, BROWSER_NAVIGATE, BROWSER_SCREENSHOT, BROWSER_SCROLL, BROWSER_TYPE_TEXT, BROWSER_WEB_SEARCH};

mod common;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt().with_env_filter("info").init();

    info!("🌐 Browser Tools Comprehensive Demo\n");
    info!("Demonstrating all 9 public browser tools\n");

    // Connect to local browser HTTP server
    let (conn, mut server) = common::connect_to_local_http_server().await?;

    // Wrap client with logging
    let workspace_root = common::find_workspace_root()
        .context("Failed to find workspace root")?;
    let log_path = workspace_root.join("tmp/mcp-client/browser.log");
    let client = common::LoggingClient::new(conn.client(), log_path)
        .await
        .context("Failed to create logging client")?;

    // Run all workflows
    let result = run_all_workflows(&client).await;

    // Always close connection
    conn.close().await?;
    server.shutdown().await?;

    result
}

async fn run_all_workflows(client: &common::LoggingClient) -> Result<()> {
    // ========================================================================
    // Workflow 1: docs.rs Search - 7 Tools
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ Workflow 1: docs.rs Search                              ║");
    info!("║ Tools: navigate, click, type_text, extract_text,        ║");
    info!("║        scroll, screenshot                                ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    // Step 1: Navigate to docs.rs
    info!("1️⃣  browser_navigate → docs.rs");
    client
        .call_tool(
            BROWSER_NAVIGATE,
            json!({
                "url": "https://docs.rs",
                "wait_for_selector": "#search"
            }),
        )
        .await?;
    info!("   ✓ Navigated to docs.rs\n");

    // Step 2: Type search query (input is autofocused, no need to click)
    info!("2️⃣  browser_type_text → \"async\"");
    client
        .call_tool(
            kodegen_config::BROWSER_TYPE_TEXT,
            json!({
                "selector": "#search",
                "text": "async"
            }),
        )
        .await?;
    info!("   ✓ Typed search query\n");

    // Step 3: Click submit/search button (triggers navigation - must wait)
    info!("3️⃣  browser_click → Submit button");
    client
        .call_tool(
            kodegen_config::BROWSER_CLICK,
            json!({
                "selector": "button[type=\"submit\"], .search-button, form button",
                "wait_for_navigation": true
            }),
        )
        .await?;
    info!("   ✓ Submitted search\n");

    // Step 4: Extract search results
    info!("4️⃣  browser_extract_text → Search results");
    let result = client.call_tool(kodegen_config::BROWSER_EXTRACT_TEXT, json!({})).await?;

    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        let response: serde_json::Value = serde_json::from_str(&text.text)?;
        let extracted = response.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let preview = if extracted.len() > 200 {
            format!("{}...", &extracted[..200])
        } else {
            extracted.to_string()
        };
        info!("   ✓ Extracted {} chars", extracted.len());
        info!("   Preview: {}\n", preview);
    }

    // Step 5: Scroll down
    info!("5️⃣  browser_scroll → Scroll down 500px");
    client
        .call_tool(
            kodegen_config::BROWSER_SCROLL,
            json!({
                "y": 500
            }),
        )
        .await?;
    info!("   ✓ Scrolled down\n");

    // Step 6: Take screenshot
    info!("6️⃣  browser_screenshot → Capture results");
    let result = client
        .call_tool(
            kodegen_config::BROWSER_SCREENSHOT,
            json!({
                "format": "png"
            }),
        )
        .await?;

    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        let response: serde_json::Value = serde_json::from_str(&text.text)?;
        let size = response
            .get("size_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        info!("   ✓ Screenshot: {} bytes\n", size);
    }

    // ========================================================================
    // Workflow 2: Web Search - 1 Tool
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ Workflow 2: Web Search (DuckDuckGo)                     ║");
    info!("║ Tool: web_search                                         ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    info!("8️⃣  web_search → \"Rust MCP server examples\"");
    let result = client
        .call_tool(
            BROWSER_WEB_SEARCH,
            json!({
                "query": "Rust MCP server examples"
            }),
        )
        .await?;

    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        let response: serde_json::Value = serde_json::from_str(&text.text)?;
        let result_count = response
            .get("result_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        info!("   ✓ Found {} search results", result_count);

        if let Some(results) = response.get("results").and_then(|v| v.as_array()) {
            info!("   Top 3 results:");
            for (i, r) in results.iter().take(3).enumerate() {
                let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("Unknown");
                let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("Unknown");
                info!("   {}. {} - {}", i + 1, title, url);
            }
        }
    }
    info!("");

    // ========================================================================
    // Workflow 3: AI-Powered Research - Async Session Pattern (3 tools)
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ Workflow 3: AI-Powered Deep Research (Async)            ║");
    info!("║ Tools: start_browser_research, get_research_status,     ║");
    info!("║        get_research_result                               ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    info!("9️⃣  start_browser_research → \"precedent setting USA Antitrust cases\"");
    info!("   (Starts background research session)\n");

    let start_result = client
        .call_tool(
            "start_browser_research",
            json!({
                "query": "precedent setting USA Antitrust cases",
                "max_pages": 5
            }),
        )
        .await?;

    let session_id = if let Some(content) = start_result.content.first()
        && let Some(text) = content.as_text()
    {
        let response: serde_json::Value = serde_json::from_str(&text.text)?;
        response.get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No session_id in response"))?
            .to_string()
    } else {
        return Err(anyhow::anyhow!("Invalid start_browser_research response"));
    };

    info!("   ✓ Research session started: {}", session_id);
    info!("   ⏳ Polling for completion (this takes 2-5 minutes)...\n");

    // Poll for completion
    let mut poll_count = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        poll_count += 1;

        let status_result = client
            .call_tool(
                "get_research_status",
                json!({
                    "session_id": session_id
                }),
            )
            .await?;

        if let Some(content) = status_result.content.first()
            && let Some(text) = content.as_text()
        {
            let status: serde_json::Value = serde_json::from_str(&text.text)?;
            let state = status.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");

            info!("   [{:02}] Status: {} ({}s elapsed)", poll_count, state, poll_count * 10);

            if state == "completed" {
                info!("   ✓ Research complete!\n");
                break;
            } else if state == "failed" {
                let error = status.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                return Err(anyhow::anyhow!("Research failed: {}", error));
            }
        }

        // Safety timeout: 30 minutes max
        if poll_count >= 180 {
            return Err(anyhow::anyhow!("Research session timed out after 30 minutes"));
        }
    }

    // Get final results
    let result = client
        .call_tool(
            "get_research_result",
            json!({
                "session_id": session_id
            }),
        )
        .await?;

    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        let response: serde_json::Value = serde_json::from_str(&text.text)?;

        if let Some(summary) = response.get("comprehensive_summary").and_then(|v| v.as_str()) {
            info!("   ✓ AI Research Report:");
            // Show first 500 chars of summary
            let preview = if summary.len() > 500 {
                format!("{}...", &summary[..500])
            } else {
                summary.to_string()
            };
            info!("\n{}\n", preview);
        }

        if let Some(sources) = response.get("sources").and_then(|v| v.as_array()) {
            info!("   📚 Sources ({} pages):", sources.len());
            for (i, source) in sources.iter().enumerate().take(5) {
                if let Some(url) = source.as_str() {
                    info!("   {}. {}", i + 1, url);
                }
            }
        }
    }
    info!("");

    // ========================================================================
    // Workflow 4: Autonomous Browser Agent - 1 Tool
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ Workflow 4: Autonomous AI Agent                         ║");
    info!("║ Tool: browser_agent                                      ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    info!("🔟  browser_agent → Compare axum vs actix-web");
    info!("   (AI autonomously navigates and extracts data)\n");

    let result = client
        .call_tool(
            kodegen_config::BROWSER_AGENT,
            json!({
                "task": "Compare axum vs actix-web crates on crates.io - find downloads, latest version, and key features for each",
                "start_url": "https://crates.io",
                "max_steps": 10,
                "temperature": 0.3
            }),
        )
        .await?;

    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        let response: serde_json::Value = serde_json::from_str(&text.text)?;

        let success = response.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
        let steps_taken = response.get("steps_taken").and_then(|v| v.as_u64()).unwrap_or(0);

        info!("   {} Agent completed in {} steps",
            if success { "✓" } else { "⚠" },
            steps_taken
        );

        if let Some(final_result) = response.get("final_result").and_then(|v| v.as_str()) {
            info!("\n   Result:\n{}\n", final_result);
        }

        if let Some(actions) = response.get("actions").and_then(|v| v.as_array()) {
            info!("   Actions taken:");
            for action in actions {
                if let Some(step) = action.get("step").and_then(|v| v.as_u64())
                    && let Some(summary) = action.get("summary").and_then(|v| v.as_str())
                {
                    info!("   Step {}: {}", step, summary);
                }
            }
        }
    }
    info!("");

    // ========================================================================
    // Summary
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ ✅ All 9 Browser Tools Demonstrated                      ║");
    info!("╠══════════════════════════════════════════════════════════╣");
    info!("║ Core Automation (6 tools):                              ║");
    info!("║   ✓ browser_navigate    ✓ browser_click                 ║");
    info!("║   ✓ browser_type_text   ✓ browser_extract_text          ║");
    info!("║   ✓ browser_scroll      ✓ browser_screenshot            ║");
    info!("║                                                          ║");
    info!("║ Advanced Tools (3 tools):                               ║");
    info!("║   ✓ web_search          ✓ browser_research              ║");
    info!("║   ✓ browser_agent                                        ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    Ok(())
}
