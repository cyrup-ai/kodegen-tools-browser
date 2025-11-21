//! `browser_web_search` MCP tool implementation
//!
//! Performs web searches and returns structured results with titles, URLs, and snippets.

use kodegen_mcp_schema::browser::BROWSER_WEB_SEARCH;
use kodegen_mcp_schema::citescrape::{WebSearchArgs, WebSearchPromptArgs};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;

// =============================================================================
// Tool Struct
// =============================================================================

#[derive(Clone)]
pub struct BrowserWebSearchTool;

impl BrowserWebSearchTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserWebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tool Trait Implementation
// =============================================================================

impl Tool for BrowserWebSearchTool {
    type Args = WebSearchArgs;
    type PromptArgs = WebSearchPromptArgs;

    fn name() -> &'static str {
        BROWSER_WEB_SEARCH
    }

    fn description() -> &'static str {
        "Perform a web search using DuckDuckGo and return structured results with titles, URLs, and snippets.\\n\\n\
         Returns up to 10 search results with:\\n\
         - rank: Result position (1-10)\\n\
         - title: Page title\\n\
         - url: Page URL\\n\
         - snippet: Description excerpt\\n\\n\
         Uses DuckDuckGo to avoid CAPTCHA issues. First search takes ~5-6s (browser launch), \
         subsequent searches take ~3-4s.\\n\\n\
         Example: web_search({\\\"query\\\": \\\"rust async programming\\\"})"
    }

    fn read_only() -> bool {
        true
    }

    fn destructive() -> bool {
        false
    }

    fn open_world() -> bool {
        true
    }

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<Vec<Content>, McpError> {
        // Validate query is not empty
        if args.query.trim().is_empty() {
            return Err(McpError::invalid_arguments("Search query cannot be empty"));
        }

        // Get global browser manager
        let browser_manager = crate::BrowserManager::global();

        // Perform search
        let results = crate::web_search::search_with_manager(&browser_manager, args.query)
            .await
            .map_err(McpError::Other)?;

        // Convert to Vec<Content> response
        let mut contents = Vec::new();

        // Terminal summary
        let summary = if results.results.is_empty() {
            format!(
                "\x1b[36m󰋱 Web Search: {}\x1b[0m\n 󰈙 Results: 0 · Top: none",
                results.query
            )
        } else {
            let first_title = results.results.first()
                .map_or("none", |r| r.title.as_str());

            format!(
                "\x1b[36m󰋱 Web Search: {}\x1b[0m\n 󰈙 Results: {} · Top: {}",
                results.query,
                results.results.len(),
                first_title
            )
        };
        contents.push(Content::text(summary));

        // JSON metadata
        let metadata = json!({
            "query": results.query,
            "result_count": results.results.len(),
            "results": results.results.iter().map(|r| json!({
                "rank": r.rank,
                "title": r.title,
                "url": r.url,
                "snippet": r.snippet,
            })).collect::<Vec<_>>(),
        });
        let json_str = match serde_json::to_string_pretty(&metadata) {
            Ok(s) => s,
            Err(_) => "{}".to_string(),
        };
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
                content: PromptMessageContent::text("How do I search the web?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The web_search tool performs web searches and returns structured results:\\n\\n\
                     **Basic usage:**\\n\
                     ```json\\n\
                     web_search({\\\"query\\\": \\\"rust async programming\\\"})\\n\
                     ```\\n\\n\
                     **Response format:**\\n\
                     ```json\\n\
                     {\\n\
                       \\\"query\\\": \\\"rust async programming\\\",\\n\
                       \\\"result_count\\\": 10,\\n\
                       \\\"results\\\": [\\n\
                         {\\n\
                           \\\"rank\\\": 1,\\n\
                           \\\"title\\\": \\\"Async Programming in Rust\\\",\\n\
                           \\\"url\\\": \\\"https://rust-lang.org/async\\\",\\n\
                           \\\"snippet\\\": \\\"Learn about async/await in Rust...\\\"\\n\
                         }\\n\
                       ]\\n\
                     }\\n\
                     ```\\n\\n\
                     **Key features:**\\n\
                     - Returns up to 10 results\\n\
                     - Includes title, URL, and description snippet\\n\
                     - Results ranked by relevance\\n\
                     - Automatic retry with exponential backoff\\n\
                     - Stealth browser configuration to avoid bot detection\\n\\n\
                     **Use cases:**\\n\
                     - Research technical topics\\n\
                     - Find documentation and tutorials\\n\
                     - Gather information for code generation\\n\
                     - Discover relevant libraries and tools",
                ),
            },
        ])
    }
}
