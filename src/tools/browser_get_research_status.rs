//! `browser_get_research_status` MCP tool implementation
//!
//! Retrieves current status and progress of a browser research session.

use crate::research::{ResearchSessionManager, ResearchStatus};
use kodegen_mcp_schema::browser::{GetResearchStatusArgs, GetResearchStatusPromptArgs};
use kodegen_mcp_tool::{Tool, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;

// =============================================================================
// Tool Struct
// =============================================================================

#[derive(Clone)]
pub struct BrowserGetResearchStatusTool;

impl BrowserGetResearchStatusTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserGetResearchStatusTool {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tool Trait Implementation
// =============================================================================

impl Tool for BrowserGetResearchStatusTool {
    type Args = GetResearchStatusArgs;
    type PromptArgs = GetResearchStatusPromptArgs;

    fn name() -> &'static str {
        "browser_get_research_status"
    }

    fn description() -> &'static str {
        "Get current status and progress of a browser research session.\n\n\
         Returns status (running/completed/failed/cancelled), runtime, pages visited, and current step.\n\
         Poll this endpoint every 5-10 seconds to monitor long-running research.\n\n\
         Example: get_research_status({\"session_id\": \"550e8400-e29b-41d4-a716-446655440000\"})"
    }

    fn read_only() -> bool {
        true // Only reads session state
    }

    fn destructive() -> bool {
        false
    }

    fn open_world() -> bool {
        false
    }

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
        // Get global session manager
        let manager = ResearchSessionManager::global();

        // Get session
        let session_ref = manager.get_session(&args.session_id)
            .await
            .map_err(|e| McpError::invalid_arguments(format!(
                "Session not found: {}. Use list_research_sessions to see active sessions.", e
            )))?;

        // Lock and read session state
        let session = session_ref.lock().await;

        // Build response with all progress information
        let results_so_far = session.total_results.load(std::sync::atomic::Ordering::Acquire);
        let has_result = session.is_complete();
        
        let mut contents = Vec::new();

        // Terminal summary
        let status_icon = match session.status {
            ResearchStatus::Running => "⏳",
            ResearchStatus::Completed => "✓",
            ResearchStatus::Failed => "✗",
            ResearchStatus::Cancelled => "⚠",
        };
        
        let pages_visited = session.progress.last().map(|p| p.pages_visited).unwrap_or(0);
        let current_step = session.progress.last().map(|p| p.message.clone()).unwrap_or_default();
        
        let summary = format!(
            "{} Research status: {:?}\n\n\
             Query: {}\n\
             Session ID: {}\n\
             Runtime: {}s\n\
             Pages visited: {}\n\
             Results collected: {}\n\
             Current step: {}",
            status_icon,
            session.status,
            session.query,
            session.session_id,
            session.runtime_seconds(),
            pages_visited,
            results_so_far,
            current_step
        );
        contents.push(Content::text(summary));

        // JSON metadata
        let metadata = json!({
            "session_id": session.session_id,
            "query": session.query,
            "status": session.status,
            "runtime_seconds": session.runtime_seconds(),
            "pages_visited": pages_visited,
            "current_step": current_step,
            "total_steps": session.progress.len(),
            "progress_history": session.progress.iter().map(|step| json!({
                "timestamp": step.timestamp,
                "message": step.message,
                "pages_visited": step.pages_visited,
            })).collect::<Vec<_>>(),
            "has_result": has_result,
            "results_so_far": results_so_far,
            "has_error": session.error.is_some(),
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
                content: PromptMessageContent::text("How do I check research progress?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use get_research_status to monitor progress of async research:\n\n\
                     **Basic usage:**\n\
                     ```json\n\
                     get_research_status({\"session_id\": \"550e8400-e29b-41d4-a716-446655440000\"})\n\
                     ```\n\n\
                     **Response while running:**\n\
                     ```json\n\
                     {\n\
                       \"session_id\": \"550e8400-...\",\n\
                       \"query\": \"Rust async best practices\",\n\
                       \"status\": \"running\",\n\
                       \"runtime_seconds\": 45,\n\
                       \"pages_visited\": 3,\n\
                       \"current_step\": \"Analyzing page 3 of 5...\",\n\
                       \"total_steps\": 4,\n\
                       \"progress_history\": [\n\
                         {\"timestamp\": 1234567890, \"message\": \"Research session started\", \"pages_visited\": 0},\n\
                         {\"timestamp\": 1234567920, \"message\": \"Found 10 search results\", \"pages_visited\": 0},\n\
                         {\"timestamp\": 1234567935, \"message\": \"Analyzing page 1 of 5...\", \"pages_visited\": 1}\n\
                       ],\n\
                       \"has_result\": false,\n\
                       \"has_error\": false\n\
                     }\n\
                     ```\n\n\
                     **Response when completed:**\n\
                     ```json\n\
                     {\n\
                       \"status\": \"completed\",\n\
                       \"runtime_seconds\": 120,\n\
                       \"pages_visited\": 5,\n\
                       \"has_result\": true,\n\
                       \"has_error\": false\n\
                     }\n\
                     ```\n\n\
                     **Polling pattern:**\n\
                     ```javascript\n\
                     while (true) {\n\
                       let status = get_research_status({session_id: id})\n\
                       \n\
                       if (status.status === \"completed\") {\n\
                         // Call get_research_result to get final data\n\
                         break\n\
                       }\n\
                       \n\
                       if (status.status === \"failed\") {\n\
                         console.log(\"Research failed\")\n\
                         break\n\
                       }\n\
                       \n\
                       // Still running, wait before polling again\n\
                       await sleep(10000) // Poll every 10 seconds\n\
                     }\n\
                     ```\n\n\
                     **Status values:**\n\
                     - `running`: Research in progress\n\
                     - `completed`: Research finished successfully\n\
                     - `failed`: Research encountered an error\n\
                     - `cancelled`: Research was stopped by user",
                ),
            },
        ])
    }
}
