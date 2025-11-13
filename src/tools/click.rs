//! Browser click tool - clicks elements by CSS selector

use kodegen_mcp_schema::browser::{BrowserClickArgs, BrowserClickPromptArgs};
use kodegen_mcp_tool::{Tool, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;
use std::sync::Arc;

use crate::manager::BrowserManager;
use crate::utils::validate_interaction_timeout;

#[derive(Clone)]
pub struct BrowserClickTool {
    manager: Arc<BrowserManager>,
}

impl BrowserClickTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

impl Tool for BrowserClickTool {
    type Args = BrowserClickArgs;
    type PromptArgs = BrowserClickPromptArgs;

    fn name() -> &'static str {
        "browser_click"
    }

    fn description() -> &'static str {
        "Click an element on the page using a CSS selector.\\n\\n\
         Automatically scrolls element into view before clicking.\\n\\n\
         Example: browser_click({\\\"selector\\\": \\\"#submit-button\\\"})\\n\
         Example: browser_click({\\\"selector\\\": \\\"button[type='submit']\\\"})"
    }

    fn read_only() -> bool {
        false // Clicking changes page state
    }

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
        // Validate selector not empty
        if args.selector.trim().is_empty() {
            return Err(McpError::invalid_arguments("Selector cannot be empty"));
        }

        // Get or create browser instance
        let browser_arc = self
            .manager
            .get_or_launch()
            .await
            .map_err(|e| McpError::Other(anyhow::anyhow!("Browser error: {}", e)))?;

        let browser_guard = browser_arc.lock().await;
        let wrapper = browser_guard.as_ref().ok_or_else(|| {
            McpError::Other(anyhow::anyhow!(
                "Browser not available. This is an internal error - please report it."
            ))
        })?;

        // Get current page (must call browser_navigate first)
        let page = crate::browser::get_current_page(wrapper)
            .await
            .map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Failed to get page. Did you call browser_navigate first? Error: {}",
                    e
                ))
            })?;

        // Find element with polling (waits for SPAs to render)
        let timeout = validate_interaction_timeout(args.timeout_ms, 5000)?;
        let element = crate::utils::wait_for_element(&page, &args.selector, timeout).await?;

        // Scroll element into view to ensure it's visible (pattern from chromiumoxide element.rs:269)
        element.scroll_into_view().await.map_err(|e| {
            McpError::Other(anyhow::anyhow!(
                "Failed to scroll element into view for selector '{}'. Error: {}",
                args.selector,
                e
            ))
        })?;

        // Get clickable point and click directly (bypasses IntersectionObserver hang)
        let point = element.clickable_point().await.map_err(|e| {
            McpError::Other(anyhow::anyhow!(
                "Failed to get clickable point for selector '{}'. \
                 Element may not be visible. Error: {}",
                args.selector,
                e
            ))
        })?;

        page.click(point).await.map_err(|e| {
            McpError::Other(anyhow::anyhow!(
                "Click failed for selector '{}'. \
                 Possible causes: (1) Element is obscured by another element, \
                 (2) Element is disabled, \
                 (3) Page changed after finding element. \
                 Error: {}",
                args.selector,
                e
            ))
        })?;

        // Wait for navigation if requested (for submit buttons, links, etc.)
        if args.wait_for_navigation.unwrap_or(false) {
            page.wait_for_navigation().await.map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Navigation after click failed for selector '{}'. Error: {}",
                    args.selector,
                    e
                ))
            })?;
        }

        let mut contents = Vec::new();

        // Terminal summary
        let summary = format!(
            "✓ Element clicked\n\n\
             Selector: {}\n\
             Navigation: {}",
            args.selector,
            if args.wait_for_navigation.unwrap_or(false) { "waited" } else { "immediate" }
        );
        contents.push(Content::text(summary));

        // JSON metadata
        let metadata = json!({
            "success": true,
            "selector": args.selector,
            "navigation_waited": args.wait_for_navigation.unwrap_or(false),
            "message": format!("Clicked element: {}", args.selector)
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
                content: PromptMessageContent::text("How do I click a button?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use browser_click with a CSS selector. Examples:\\n\
                     - browser_click({\\\"selector\\\": \\\"#submit\\\"}) - By ID\\n\
                     - browser_click({\\\"selector\\\": \\\".btn-primary\\\"}) - By class\\n\
                     - browser_click({\\\"selector\\\": \\\"button[type='submit']\\\"}) - By attribute\\n\
                     - browser_click({\\\"selector\\\": \\\"form button:first-child\\\"}) - Complex selector",
                ),
            },
        ])
    }
}
