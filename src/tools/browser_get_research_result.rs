//! `browser_get_research_result` MCP tool implementation
//!
//! Retrieves final results from a completed browser research session.

use crate::research::{ResearchSessionManager, ResearchStatus};
use kodegen_mcp_schema::browser::{GetResearchResultArgs, GetResearchResultPromptArgs, BROWSER_GET_RESEARCH_RESULT};
use kodegen_mcp_tool::{Tool, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;

// =============================================================================
// Tool Struct
// =============================================================================

#[derive(Clone)]
pub struct BrowserGetResearchResultTool;

impl Default for BrowserGetResearchResultTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserGetResearchResultTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

// =============================================================================
// Tool Trait Implementation
// =============================================================================

impl Tool for BrowserGetResearchResultTool {
    type Args = GetResearchResultArgs;
    type PromptArgs = GetResearchResultPromptArgs;

    fn name() -> &'static str {
        BROWSER_GET_RESEARCH_RESULT
    }

    fn description() -> &'static str {
        "Get final results from a completed browser research session.\n\n\
         Returns comprehensive summary, sources, key findings, and individual page results.\n\
         Only works when research status is 'completed'. Check status with get_research_status first.\n\n\
         Example: get_research_result({\"session_id\": \"550e8400-e29b-41d4-a716-446655440000\"})"
    }

    fn read_only() -> bool {
        true // Only reads session results
    }

    fn destructive() -> bool {
        false
    }

    fn open_world() -> bool {
        false
    }

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
        let session_id = args.session_id;

        // Get global session manager
        let manager = ResearchSessionManager::global();

        // Get session
        let session_ref = manager.get_session(&session_id)
            .await
            .map_err(|e| McpError::invalid_arguments(format!(
                "Session not found: {}. Use list_research_sessions to see active sessions.", e
            )))?;

        // Lock and read session state
        let session = session_ref.lock().await;

        // Check status
        match session.status {
            ResearchStatus::Running => {
                Err(McpError::invalid_arguments(
                    format!("Research is still running ({} seconds). Use get_research_status to monitor progress.",
                        session.runtime_seconds())
                ))
            }
            ResearchStatus::Failed => {
                Err(McpError::Other(anyhow::anyhow!(
                    "Research failed: {}",
                    session.error.as_deref().unwrap_or("Unknown error")
                )))
            }
            ResearchStatus::Cancelled => {
                Err(McpError::Other(anyhow::anyhow!(
                    "Research was cancelled after {} seconds",
                    session.runtime_seconds()
                )))
            }
            ResearchStatus::Completed => {
                // Build result JSON from incremental results vector
                let results_guard = session.results.read().await;
                let results = results_guard.clone(); // Clone to avoid holding lock
                let query = session.query.clone();
                let session_id = session.session_id.clone();
                let runtime_seconds = session.runtime_seconds();
                drop(results_guard);
                drop(session); // Drop session lock before expensive processing
                
                if results.is_empty() {
                    Err(McpError::Other(anyhow::anyhow!(
                        "Research completed but no results available"
                    )))
                } else {
                    // Build comprehensive summary (moved from start_browser_research.rs)
                    let pages_visited = results.len();
                    let mut comprehensive_summary = format!("# Research Report: {}\n\n", query);
                    comprehensive_summary.push_str(&format!("Analyzed {} pages\n\n", pages_visited));
                    
                    for (i, result) in results.iter().enumerate() {
                        comprehensive_summary.push_str(&format!("## Source {} - {}\n", i + 1, result.title));
                        comprehensive_summary.push_str(&format!("URL: {}\n\n", result.url));
                        comprehensive_summary.push_str(&result.summary);
                        comprehensive_summary.push_str("\n\n---\n\n");
                    }
                    
                    let key_findings: Vec<String> = results
                        .iter()
                        .filter_map(|r| {
                            let first_line = r.summary.lines().next()?;
                            if !first_line.is_empty() {
                                Some(format!("{}: {}", r.title, first_line))
                            } else {
                                None
                            }
                        })
                        .collect();
                    
                    let sources: Vec<String> = results.iter().map(|r| r.url.clone()).collect();
                    
                    let result_json = json!({
                        "success": true,
                        "query": query,
                        "pages_visited": pages_visited,
                        "comprehensive_summary": comprehensive_summary,
                        "sources": sources,
                        "key_findings": key_findings,
                        "individual_results": results.iter().map(|r| json!({
                            "url": r.url,
                            "title": r.title,
                            "summary": r.summary,
                            "content_length": r.content.len(),
                            "timestamp": r.timestamp.to_rfc3339(),
                        })).collect::<Vec<_>>(),
                    });
                    
                    let mut contents = Vec::new();

                    // Terminal summary - compact 2-line format with ANSI colors and Nerd Font icons
                    let summary = format!(
                        "\x1b[36m󰧞 Research Results: {}\x1b[0m",
                        query
                    );

                    let metadata_line = format!(
                        " 󰌋 Session: {} · Pages: {} · Sources: {}",
                        session_id,
                        pages_visited,
                        sources.len()
                    );

                    let summary_text = format!("{}\n{}", summary, metadata_line);
                    contents.push(Content::text(summary_text));

                    // JSON metadata
                    let metadata = json!({
                        "session_id": session_id,
                        "query": query,
                        "status": "completed",
                        "runtime_seconds": runtime_seconds,
                        "result": result_json,
                    });
                    let json_str = serde_json::to_string_pretty(&metadata)
                        .unwrap_or_else(|_| "{}".to_string());
                    contents.push(Content::text(json_str));

                    Ok(contents)
                }
            }
        }
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I get research results?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use get_research_result to retrieve final results from completed research:\n\n\
                     **Basic usage:**\n\
                     ```json\n\
                     get_research_result({\"session_id\": \"550e8400-e29b-41d4-a716-446655440000\"})\n\
                     ```\n\n\
                     **Response format:**\n\
                     ```json\n\
                     {\n\
                       \"session_id\": \"550e8400-...\",\n\
                       \"query\": \"Rust async best practices\",\n\
                       \"status\": \"completed\",\n\
                       \"runtime_seconds\": 120,\n\
                       \"result\": {\n\
                         \"success\": true,\n\
                         \"pages_visited\": 5,\n\
                         \"comprehensive_summary\": \"# Research Report: Rust async best practices\\n\\n...\",\n\
                         \"sources\": [\n\
                           \"https://rust-lang.org/async\",\n\
                           \"https://tokio.rs/tutorial\"\n\
                         ],\n\
                         \"key_findings\": [\n\
                           \"Use async/await for concurrent operations\",\n\
                           \"Tokio is the most popular async runtime\"\n\
                         ],\n\
                         \"individual_results\": [\n\
                           {\n\
                             \"url\": \"https://rust-lang.org/async\",\n\
                             \"title\": \"Async Programming in Rust\",\n\
                             \"summary\": \"Detailed explanation of async...\",\n\
                             \"content_length\": 12500,\n\
                             \"timestamp\": \"2024-03-15T10:30:00Z\"\n\
                           }\n\
                         ]\n\
                       }\n\
                     }\n\
                     ```\n\n\
                     **Complete workflow:**\n\
                     ```javascript\n\
                     // 1. Start research\n\
                     const session = start_browser_research({\n\
                       query: \"Rust async best practices\",\n\
                       max_pages: 5\n\
                     })\n\
                     \n\
                     // 2. Poll for completion\n\
                     let status\n\
                     while (true) {\n\
                       status = get_research_status({session_id: session.session_id})\n\
                       if (status.status === \"completed\") break\n\
                       if (status.status === \"failed\") throw new Error(\"Research failed\")\n\
                       await sleep(10000)\n\
                     }\n\
                     \n\
                     // 3. Get results\n\
                     const results = get_research_result({session_id: session.session_id})\n\
                     console.log(results.result.comprehensive_summary)\n\
                     ```\n\n\
                     **Error handling:**\n\
                     - If research is still running, you'll get an error prompting to check status\n\
                     - If research failed, you'll get the error message\n\
                     - If research was cancelled, you'll get a cancellation notice\n\
                     - Only completed research returns results",
                ),
            },
        ])
    }
}
