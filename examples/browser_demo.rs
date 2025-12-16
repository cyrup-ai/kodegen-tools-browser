//! Browser tools comprehensive demonstration
//!
//! Demonstrates all 8 public browser tools using real-world examples:
//! - Workflow 1: docs.rs search (7 tools)
//! - Workflow 2: AI research (1 tool)
//! - Workflow 3: Autonomous agent (1 tool)

use anyhow::{Context, Result};
use serde_json::json;
use tracing::info;
use kodegen_config::{BROWSER_AGENT, BROWSER_CLICK, BROWSER_EXTRACT_TEXT, BROWSER_NAVIGATE, BROWSER_SCREENSHOT, BROWSER_SCROLL, BROWSER_TYPE_TEXT};

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
                "wait_for_selector": "input[name=\"query\"]"
            }),
        )
        .await?;
    info!("   ✓ Navigated to docs.rs\n");

    // Step 2: Type search query
    info!("2️⃣  browser_type_text → \"async\"");
    client
        .call_tool(
            BROWSER_TYPE_TEXT,
            json!({
                "selector": "input[name=\"query\"]",
                "text": "async"
            }),
        )
        .await?;
    info!("   ✓ Typed search query\n");

    // Step 3: Click submit/search button (triggers navigation - must wait)
    info!("3️⃣  browser_click → Submit button");
    client
        .call_tool(
            BROWSER_CLICK,
            json!({
                "selector": "button[type=\"submit\"]",
                "wait_for_navigation": true
            }),
        )
        .await?;
    info!("   ✓ Submitted search\n");

    // Step 4: Extract search results
    info!("4️⃣  browser_extract_text → Search results");
    let result = client.call_tool(BROWSER_EXTRACT_TEXT, json!({})).await?;

    // Content layout: [0]=branded line, [1]=display, [2]=JSON metadata
    // Or without branding: [0]=display, [1]=JSON metadata
    // Find the JSON metadata by trying to parse each content item
    let response: Option<serde_json::Value> = result.content.iter().rev().find_map(|c| {
        c.as_text()
            .and_then(|t| serde_json::from_str(&t.text).ok())
    });

    if let Some(response) = response {
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
            BROWSER_SCROLL,
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
            BROWSER_SCREENSHOT,
            json!({
                "format": "png"
            }),
        )
        .await?;

    // Content layout: [0]=branded line, [1]=display, [2]=JSON metadata
    // Or without branding: [0]=display, [1]=JSON metadata
    // Find the JSON metadata by trying to parse each content item
    let response: Option<serde_json::Value> = result.content.iter().rev().find_map(|c| {
        c.as_text()
            .and_then(|t| serde_json::from_str(&t.text).ok())
    });

    if let Some(response) = response {
        let size = response
            .get("size_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        info!("   ✓ Screenshot: {} bytes\n", size);
    }

    // ========================================================================
    // Workflow 2: AI-Powered Research - Action-based API
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ Workflow 2: AI-Powered Deep Research                    ║");
    info!("║ Tool: browser_research (action: RESEARCH/READ/LIST/KILL)║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    info!("8️⃣  browser_research RESEARCH → \"Rust async programming best practices\"");
    info!("   (Starts background research session)\n");

    // Start research with short timeout to return immediately
    let start_result = client
        .call_tool(
            "browser_research",
            json!({
                "action": "RESEARCH",
                "query": "Rust async programming best practices",
                "max_pages": 3,
                "session": 0,
                "await_completion_ms": 0  // Fire-and-forget, return immediately
            }),
        )
        .await?;

    if let Some(content) = start_result.content.first()
        && let Some(text) = content.as_text()
    {
        let response: serde_json::Value = serde_json::from_str(&text.text)?;
        let session = response.get("session").and_then(|v| v.as_u64()).unwrap_or(0);
        let status = response.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
        info!("   ✓ Research session {} started (status: {})", session, status);
    }
    
    info!("   ⏳ Polling for completion...\n");

    // Poll for completion using READ action
    let mut poll_count = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        poll_count += 1;

        let status_result = client
            .call_tool(
                "browser_research",
                json!({
                    "action": "READ",
                    "session": 0
                }),
            )
            .await?;

        if let Some(content) = status_result.content.first()
            && let Some(text) = content.as_text()
        {
            let response: serde_json::Value = serde_json::from_str(&text.text)?;
            let status = response.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            let pages = response.get("pages_analyzed").and_then(|v| v.as_u64()).unwrap_or(0);
            let completed = response.get("completed").and_then(|v| v.as_bool()).unwrap_or(false);

            info!("   [{:02}] Status: {} | Pages: {} ({}s elapsed)", 
                poll_count, status, pages, poll_count * 5);

            if completed {
                info!("   ✓ Research complete!\n");
                
                // Show results
                if let Some(summary) = response.get("summary").and_then(|v| v.as_str()) {
                    info!("   ✓ AI Research Summary:");
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
                        let url = source.get("url").and_then(|v| v.as_str()).unwrap_or("Unknown");
                        let title = source.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
                        info!("   {}. {} - {}", i + 1, title, url);
                    }
                }
                break;
            }
            
            // Check for error
            if let Some(error) = response.get("error").and_then(|v| v.as_str()) {
                return Err(anyhow::anyhow!("Research failed: {}", error));
            }
        }

        // Safety timeout: 5 minutes max
        if poll_count >= 60 {
            return Err(anyhow::anyhow!("Research session timed out after 5 minutes"));
        }
    }
    info!("");

    // ========================================================================
    // Workflow 3: Autonomous Browser Agent - 1 Tool
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ Workflow 3: Autonomous AI Agent                         ║");
    info!("║ Tool: browser_agent                                      ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    info!("9️⃣  browser_agent → Compare axum vs actix-web");
    info!("   (AI autonomously navigates and extracts data)\n");

    let result = client
        .call_tool(
            BROWSER_AGENT,
            json!({
                "action": "PROMPT",
                "task": "Compare axum vs actix-web crates on crates.io - find downloads, latest version, and key features for each",
                "start_url": "https://crates.io",
                "max_steps": 10,
                "temperature": 0.3
            }),
        )
        .await?;

    // DEBUG: Print raw response content
    info!("   DEBUG: Response has {} content items", result.content.len());
    for (i, content) in result.content.iter().enumerate() {
        if let Some(text) = content.as_text() {
            let preview = if text.text.len() > 200 {
                format!("{}...", &text.text[..200])
            } else {
                text.text.clone()
            };
            info!("   DEBUG: content[{}] = {}", i, preview);
        } else {
            info!("   DEBUG: content[{}] = <non-text>", i);
        }
    }

    // Content layout: [0]=branded line, [1]=display, [2]=JSON metadata
    // Or without branding: [0]=display, [1]=JSON metadata
    // Find the JSON metadata by trying to parse each content item
    let response: Option<serde_json::Value> = result.content.iter().rev().find_map(|c| {
        c.as_text()
            .and_then(|t| serde_json::from_str(&t.text).ok())
    });

    if response.is_none() {
        return Err(anyhow::anyhow!("No JSON metadata found in response"));
    }
    let response = response.unwrap();

    let completed = response.get("completed").and_then(|v| v.as_bool()).unwrap_or(false);
    let steps_taken = response.get("steps_taken").and_then(|v| v.as_u64()).unwrap_or(0);

    info!("   {} Agent completed in {} steps",
        if completed { "✓" } else { "⚠" },
        steps_taken
    );

    if let Some(summary) = response.get("summary").and_then(|v| v.as_str()) {
        info!("\n   Result:\n{}\n", summary);
    }

    if let Some(history) = response.get("history").and_then(|v| v.as_array()) {
        info!("   History:");
        for entry in history {
            if let Some(step) = entry.get("step").and_then(|v| v.as_u64())
                && let Some(step_summary) = entry.get("summary").and_then(|v| v.as_str())
            {
                info!("   Step {}: {}", step, step_summary);
            }
        }
    }
    info!("");

    // ========================================================================
    // Summary
    // ========================================================================
    info!("\n╔══════════════════════════════════════════════════════════╗");
    info!("║ ✅ All 8 Browser Tools Demonstrated                      ║");
    info!("╠══════════════════════════════════════════════════════════╣");
    info!("║ Core Automation (6 tools):                              ║");
    info!("║   ✓ browser_navigate    ✓ browser_click                 ║");
    info!("║   ✓ browser_type_text   ✓ browser_extract_text          ║");
    info!("║   ✓ browser_scroll      ✓ browser_screenshot            ║");
    info!("║                                                          ║");
    info!("║ Advanced Tools (2 tools):                               ║");
    info!("║   ✓ browser_research    ✓ browser_agent                 ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    Ok(())
}
