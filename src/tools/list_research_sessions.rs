//! `list_research_sessions` MCP tool implementation
//!
//! Lists all active browser research sessions.

use crate::research::ResearchSessionManager;
use kodegen_mcp_schema::browser::{ListResearchSessionsArgs, ListResearchSessionsPromptArgs};
use kodegen_mcp_tool::{Tool, error::McpError};
use rmcp::model::{PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::{Value, json};

// =============================================================================
// Tool Struct
// =============================================================================

#[derive(Clone)]
pub struct ListResearchSessionsTool;

impl ListResearchSessionsTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListResearchSessionsTool {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tool Trait Implementation
// =============================================================================

impl Tool for ListResearchSessionsTool {
    type Args = ListResearchSessionsArgs;
    type PromptArgs = ListResearchSessionsPromptArgs;

    fn name() -> &'static str {
        "list_research_sessions"
    }

    fn description() -> &'static str {
        "List all active browser research sessions.\n\n\
         Shows session ID, query, status, runtime, and progress for each session.\n\
         Useful for tracking multiple concurrent research tasks.\n\n\
         Example: list_research_sessions({})"
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

    async fn execute(&self, _args: Self::Args) -> Result<Value, McpError> {
        // Get global session manager
        let manager = ResearchSessionManager::global();

        // List all sessions
        let sessions = manager.list_sessions().await;

        Ok(json!({
            "total_sessions": sessions.len(),
            "sessions": sessions,
        }))
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I see all research sessions?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use list_research_sessions to see all active browser research:\n\n\
                     **Basic usage:**\n\
                     ```json\n\
                     list_research_sessions({})\n\
                     ```\n\n\
                     **Response:**\n\
                     ```json\n\
                     {\n\
                       \"total_sessions\": 2,\n\
                       \"sessions\": [\n\
                         {\n\
                           \"session_id\": \"550e8400-e29b-41d4-a716-446655440000\",\n\
                           \"query\": \"Rust async best practices\",\n\
                           \"status\": \"running\",\n\
                           \"started_at\": 1234567890,\n\
                           \"runtime_seconds\": 45,\n\
                           \"pages_visited\": 3,\n\
                           \"current_step\": \"Analyzing page 3 of 5...\"\n\
                         },\n\
                         {\n\
                           \"session_id\": \"660e8400-e29b-41d4-a716-446655440001\",\n\
                           \"query\": \"WebGPU tutorial\",\n\
                           \"status\": \"completed\",\n\
                           \"started_at\": 1234560000,\n\
                           \"runtime_seconds\": 120,\n\
                           \"pages_visited\": 8,\n\
                           \"current_step\": \"Research completed - 8 pages analyzed\"\n\
                         }\n\
                       ]\n\
                     }\n\
                     ```\n\n\
                     **Use cases:**\n\
                     - Check if any research is still running\n\
                     - Find session_id for a specific query\n\
                     - Monitor multiple concurrent research tasks\n\
                     - Debug stuck or long-running sessions\n\n\
                     **Session lifecycle:**\n\
                     1. Sessions appear when created with start_browser_research\n\
                     2. Status changes from running → completed/failed/cancelled\n\
                     3. Sessions auto-cleanup after 5 minutes of being completed\n\
                     4. Running sessions never auto-cleanup (must cancel manually)\n\n\
                     **Managing multiple sessions:**\n\
                     ```javascript\n\
                     // Start multiple research tasks\n\
                     const s1 = start_browser_research({query: \"topic A\", max_pages: 5})\n\
                     const s2 = start_browser_research({query: \"topic B\", max_pages: 5})\n\
                     const s3 = start_browser_research({query: \"topic C\", max_pages: 5})\n\
                     \n\
                     // Check all sessions\n\
                     const {sessions} = list_research_sessions({})\n\
                     \n\
                     // Wait for all to complete\n\
                     while (true) {\n\
                       const {sessions} = list_research_sessions({})\n\
                       const running = sessions.filter(s => s.status === \"running\")\n\
                       if (running.length === 0) break\n\
                       \n\
                       console.log(`${running.length} sessions still running...`)\n\
                       await sleep(10000)\n\
                     }\n\
                     \n\
                     // Collect all results\n\
                     const results = [\n\
                       get_research_result({session_id: s1.session_id}),\n\
                       get_research_result({session_id: s2.session_id}),\n\
                       get_research_result({session_id: s3.session_id})\n\
                     ]\n\
                     ```",
                ),
            },
        ])
    }
}
