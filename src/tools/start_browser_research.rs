//! `start_browser_research` MCP tool implementation
//!
//! Starts async browser research session that runs in the background.
//! Returns session_id immediately for polling progress and results.

use crate::research::{ResearchSessionManager, ResearchStatus};
use crate::utils::{DeepResearch, ResearchOptions};
use kodegen_mcp_schema::browser::{StartBrowserResearchArgs, StartBrowserResearchPromptArgs};
use kodegen_mcp_tool::{Tool, error::McpError};
use rmcp::model::{PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::{Value, json};
use std::sync::Arc;

// =============================================================================
// Tool Struct
// =============================================================================

#[derive(Clone)]
pub struct StartBrowserResearchTool;

impl StartBrowserResearchTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for StartBrowserResearchTool {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tool Trait Implementation
// =============================================================================

impl Tool for StartBrowserResearchTool {
    type Args = StartBrowserResearchArgs;
    type PromptArgs = StartBrowserResearchPromptArgs;

    fn name() -> &'static str {
        "start_browser_research"
    }

    fn description() -> &'static str {
        "Start async browser research session that runs in background.\n\n\
         Searches web, crawls multiple pages, and generates AI summaries without blocking.\n\
         Returns session_id immediately for polling status/results with get_research_status and get_research_result.\n\n\
         Research continues running for 2-5 minutes depending on max_pages.\n\
         Use list_research_sessions to see all active sessions.\n\
         Use stop_browser_research to cancel a running session.\n\n\
         Example: start_browser_research({\"query\": \"Rust async best practices\", \"max_pages\": 5})"
    }

    fn read_only() -> bool {
        false // Creates session state
    }

    fn destructive() -> bool {
        false
    }

    fn open_world() -> bool {
        true // Can research arbitrary URLs from search results
    }

    async fn execute(&self, args: Self::Args) -> Result<Value, McpError> {
        // Validate query
        if args.query.trim().is_empty() {
            return Err(McpError::invalid_arguments("Research query cannot be empty"));
        }

        // Generate unique session ID
        let session_id = uuid::Uuid::new_v4().to_string();

        // Get global session manager
        let manager = ResearchSessionManager::global();

        // Create session
        let session_ref = manager.create_session(session_id.clone(), args.query.clone())
            .await
            .map_err(|e| McpError::Other(anyhow::anyhow!(
                "Failed to create research session: {}", e
            )))?;

        // Clone session ref for background task
        let session_ref_bg = Arc::clone(&session_ref);
        let query = args.query.clone();

        // Build research options
        let options = Some(ResearchOptions {
            max_pages: args.max_pages,
            max_depth: args.max_depth,
            search_engine: args.search_engine.clone(),
            include_links: args.include_links,
            extract_tables: args.extract_tables,
            extract_images: args.extract_images,
            timeout_seconds: args.timeout_seconds,
        });

        // Clone Arc pointers for background task (matches search pattern)
        let session = session_ref.lock().await;
        let results = Arc::clone(&session.results);
        let total_results = Arc::clone(&session.total_results);
        drop(session);

        // Spawn background research task
        let task_handle = tokio::spawn(async move {
            // Get browser manager
            let browser_manager = crate::BrowserManager::global();

            // Create DeepResearch instance
            let research = DeepResearch::new(
                browser_manager,
                args.temperature,
                args.max_tokens,
            );

            // Run research (incremental streaming pattern)
            match research.research(&query, options, results, total_results.clone()).await {
                Ok(()) => {
                    // Research completed successfully
                    let mut session = session_ref_bg.lock().await;
                    let count = total_results.load(std::sync::atomic::Ordering::Acquire);
                    session.status = crate::research::ResearchStatus::Completed;
                    session.add_progress(
                        format!("Research completed - {} pages analyzed", count),
                        count
                    );
                    // Result building moved to get_research_result.rs
                }
                Err(e) => {
                    // Update session with error
                    let mut session = session_ref_bg.lock().await;
                    session.fail(format!("Research failed: {}", e));
                }
            }
        });

        // Store task handle in session
        {
            let mut session = session_ref.lock().await;
            session.task_handle = Some(task_handle);
            session.add_progress("Research session started".to_string(), 0);
        }

        // Return session ID immediately
        Ok(json!({
            "session_id": session_id,
            "status": ResearchStatus::Running,
            "query": args.query,
            "message": "Research session started. Use get_research_status to monitor progress."
        }))
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I start async browser research?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The start_browser_research tool begins long-running research in the background:\n\n\
                     **Basic usage:**\n\
                     ```json\n\
                     start_browser_research({\"query\": \"Rust async best practices\", \"max_pages\": 5})\n\
                     ```\n\n\
                     **Response:**\n\
                     ```json\n\
                     {\n\
                       \"session_id\": \"550e8400-e29b-41d4-a716-446655440000\",\n\
                       \"status\": \"running\",\n\
                       \"query\": \"Rust async best practices\",\n\
                       \"message\": \"Research session started...\"\n\
                     }\n\
                     ```\n\n\
                     **Next steps:**\n\
                     1. Save the session_id\n\
                     2. Poll with get_research_status(session_id) for progress\n\
                     3. When status=\"completed\", call get_research_result(session_id)\n\
                     4. Use stop_browser_research(session_id) to cancel if needed\n\n\
                     **Full workflow example:**\n\
                     ```\n\
                     // Start research\n\
                     let session = start_browser_research({\"query\": \"WebGPU tutorial\", \"max_pages\": 8})\n\
                     let id = session.session_id\n\
                     \n\
                     // Check status every 10 seconds\n\
                     while (true) {\n\
                       let status = get_research_status({\"session_id\": id})\n\
                       if (status.status == \"completed\") break\n\
                       sleep(10000)\n\
                     }\n\
                     \n\
                     // Get final results\n\
                     let results = get_research_result({\"session_id\": id})\n\
                     ```",
                ),
            },
        ])
    }
}
