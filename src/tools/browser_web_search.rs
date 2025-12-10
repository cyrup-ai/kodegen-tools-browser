//! `browser_web_search` MCP tool implementation
//!
//! Performs web searches and returns structured results with titles, URLs, and snippets.

use kodegen_mcp_schema::browser::{BROWSER_WEB_SEARCH, WebSearchPrompts};
use kodegen_mcp_schema::citescrape::{WebSearchArgs, WebSearchOutput};
use kodegen_mcp_schema::{Tool, ToolExecutionContext, ToolResponse, McpError};

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
    type Prompts = WebSearchPrompts;

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

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<ToolResponse<WebSearchOutput>, McpError> {
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

        // Terminal summary
        let summary = if results.results.is_empty() {
            format!(
                "\x1b[36mWeb Search: {}\x1b[0m\n Results: 0 · Top: none",
                results.query
            )
        } else {
            let first_title = results.results.first()
                .map_or("none", |r| r.title.as_str());

            format!(
                "\x1b[36mWeb Search: {}\x1b[0m\n Results: {} · Top: {}",
                results.query,
                results.results.len(),
                first_title
            )
        };

        // Build typed output using schema types
        let output = WebSearchOutput {
            success: true,
            query: results.query,
            results_count: results.results.len(),
            results: results.results.into_iter().map(|r| {
                kodegen_mcp_schema::citescrape::WebSearchResultItem {
                    rank: r.rank as u32,
                    title: r.title,
                    url: r.url,
                    snippet: Some(r.snippet),
                }
            }).collect(),
        };

        Ok(ToolResponse::new(summary, output))
    }
}
