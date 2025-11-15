//! `browser_stop_research` MCP tool implementation
//!
//! Cancels a running browser research session.

use crate::research::{ResearchSessionManager, ResearchStatus};
use kodegen_mcp_schema::browser::{StopBrowserResearchArgs, StopBrowserResearchPromptArgs, BROWSER_STOP_RESEARCH};
use kodegen_mcp_tool::{Tool, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;

// =============================================================================
// Tool Struct
// =============================================================================

#[derive(Clone)]
pub struct BrowserStopResearchTool;

impl BrowserStopResearchTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserStopResearchTool {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tool Trait Implementation
// =============================================================================

impl Tool for BrowserStopResearchTool {
    type Args = StopBrowserResearchArgs;
    type PromptArgs = StopBrowserResearchPromptArgs;

    fn name() -> &'static str {
        BROWSER_STOP_RESEARCH
    }

    fn description() -> &'static str {
        "Cancel a running browser research session.\n\n\
         Aborts the background research task and marks session as cancelled.\n\
         Does nothing if research is already completed or failed.\n\n\
         Example: stop_browser_research({\"session_id\": \"550e8400-e29b-41d4-a716-446655440000\"})"
    }

    fn read_only() -> bool {
        false // Modifies session state
    }

    fn destructive() -> bool {
        true // Cancels ongoing work
    }

    fn open_world() -> bool {
        false
    }

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
        // Get global session manager
        let manager = ResearchSessionManager::global();

        // Stop session (aborts background task)
        manager.stop_session(&args.session_id)
            .await
            .map_err(|e| McpError::invalid_arguments(format!(
                "Failed to stop session: {}. Use list_research_sessions to see active sessions.", e
            )))?;

        // Get session to report final state
        let session_ref = manager.get_session(&args.session_id).await.map_err(|e| {
            McpError::Other(anyhow::anyhow!("Session stopped but could not retrieve state: {}", e))
        })?;

        let session = session_ref.lock().await;

        let mut contents = Vec::new();

        // Terminal summary
        let pages_visited = session.progress.last().map(|p| p.pages_visited).unwrap_or(0);
        let runtime_seconds = session.runtime_seconds();
        
        let summary = format!(
            "⚠ Research cancelled\n\n\
             Session ID: {}\n\
             Query: {}\n\
             Runtime: {}s\n\
             Pages visited: {}",
            session.session_id,
            session.query,
            runtime_seconds,
            pages_visited
        );
        contents.push(Content::text(summary));

        // JSON metadata
        let metadata = json!({
            "session_id": session.session_id,
            "query": session.query,
            "status": ResearchStatus::Cancelled,
            "runtime_seconds": runtime_seconds,
            "pages_visited": pages_visited,
            "message": format!("Research cancelled after {} seconds", runtime_seconds)
        });
        let json_str = serde_json::to_string_pretty(&metadata)
            .unwrap_or_else(|_| "{}".to_string());
        contents.push(Content::text(json_str));

        Ok(contents)
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I cancel research?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use stop_browser_research to cancel a running research session:\n\n\
                     **Basic usage:**\n\
                     ```json\n\
                     stop_browser_research({\"session_id\": \"550e8400-e29b-41d4-a716-446655440000\"})\n\
                     ```\n\n\
                     **Response:**\n\
                     ```json\n\
                     {\n\
                       \"session_id\": \"550e8400-...\",\n\
                       \"query\": \"Rust async best practices\",\n\
                       \"status\": \"cancelled\",\n\
                       \"runtime_seconds\": 45,\n\
                       \"pages_visited\": 3,\n\
                       \"message\": \"Research cancelled after 45 seconds\"\n\
                     }\n\
                     ```\n\n\
                     **What happens:**\n\
                     1. Background research task is aborted immediately\n\
                     2. Session status changes to 'cancelled'\n\
                     3. Partial progress is preserved\n\
                     4. Session remains queryable for 5 minutes before cleanup\n\n\
                     **Use cases:**\n\
                     - Research is taking too long\n\
                     - Found enough information already\n\
                     - Need to start over with different parameters\n\
                     - Resource cleanup before shutdown\n\n\
                     **After cancellation:**\n\
                     - get_research_status will show status=\"cancelled\"\n\
                     - get_research_result will return an error\n\
                     - No results are available from cancelled research\n\n\
                     **Example with timeout:**\n\
                     ```javascript\n\
                     // Start research\n\
                     const session = start_browser_research({\n\
                       query: \"complex topic\",\n\
                       max_pages: 20\n\
                     })\n\
                     \n\
                     // Set 2-minute timeout\n\
                     setTimeout(() => {\n\
                       stop_browser_research({session_id: session.session_id})\n\
                       console.log(\"Research timed out\")\n\
                     }, 120000)\n\
                     \n\
                     // Poll for results\n\
                     while (true) {\n\
                       const status = get_research_status({session_id: session.session_id})\n\
                       if (status.status === \"completed\") break\n\
                       if (status.status === \"cancelled\") break\n\
                       await sleep(10000)\n\
                     }\n\
                     ```",
                ),
            },
        ])
    }
}
