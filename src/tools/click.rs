//! Browser click tool - clicks elements by CSS selector

use kodegen_mcp_schema::browser::{BrowserClickArgs, BrowserClickPromptArgs, BROWSER_CLICK};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, error::McpError};
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
        BROWSER_CLICK
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

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<Vec<Content>, McpError> {
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
            "\x1b[33m  Click: {}\x1b[0m\n \
              Element: {} · Action: clicked",
            args.selector,
            args.selector
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
        vec![PromptArgument {
            name: "selector_type".to_string(),
            title: None,
            description: Some(
                "Optional CSS selector pattern to focus on (e.g., 'id', 'class', 'attribute', 'pseudo', 'complex')"
                    .to_string(),
            ),
            required: Some(false),
        }]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "How do I click elements on a webpage using CSS selectors?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The browser_click tool clicks elements by CSS selector. The tool automatically handles \
                     scroll-into-view, waits for elements to render, and handles timeouts.\\n\\n\
                     \\x1b[1mBasic Selector Examples:\\x1b[0m\\n\
                     - By ID: browser_click({\\\"selector\\\": \\\"#submit-button\\\"})\\n\
                     - By class: browser_click({\\\"selector\\\": \\\".btn-primary\\\"})\\n\
                     - By element: browser_click({\\\"selector\\\": \\\"button\\\"})\\n\
                     - By attribute: browser_click({\\\"selector\\\": \\\"button[type='submit']\\\"})\\n\\n\
                     \\x1b[1mAdvanced Selectors:\\x1b[0m\\n\
                     - Pseudo-selector: browser_click({\\\"selector\\\": \\\"form button:first-child\\\"})\\n\
                     - Child combinator: browser_click({\\\"selector\\\": \\\".modal .close-button\\\"})\\n\
                     - Attribute contains: browser_click({\\\"selector\\\": \\\"a[href*='logout']\\\"})\\n\
                     - :nth-child: browser_click({\\\"selector\\\": \\\"tr:nth-child(3) td button\\\"})\\n\\n\
                     \\x1b[1mKey Behaviors:\\x1b[0m\\n\
                     1. Auto scroll: Element is automatically scrolled into view before clicking\\n\
                     2. Polling: Waits up to timeout_ms for element to appear (default 5000ms)\\n\
                     3. Visibility: Element must be visible/clickable (not obscured by other elements)\\n\
                     4. Disabled detection: Returns error if element is disabled\\n\
                     5. Navigation waiting: Set wait_for_navigation: true for buttons/links that navigate\\n\\n\
                     \\x1b[1mCommon Patterns:\\x1b[0m\\n\
                     - Click a submit button: browser_click({\\\"selector\\\": \\\"form button[type='submit']\\\"})\\n\
                     - Click a link: browser_click({\\\"selector\\\": \\\"a.nav-link\\\"})\\n\
                     - Click modal close: browser_click({\\\"selector\\\": \\\".modal .close-btn\\\"})\\n\
                     - Click checkbox: browser_click({\\\"selector\\\": \\\"input[type='checkbox']\\\"})\\n\
                     - Wait after click: browser_click({\\\"selector\\\": \\\"button.submit\\\", \\\"wait_for_navigation\\\": true})\\n\\n\
                     \\x1b[1mError Scenarios:\\x1b[0m\\n\
                     - Selector not found: Returns error if element doesn't exist\\n\
                     - Element obscured: Returns error if another element blocks the click\\n\
                     - Element disabled: Returns error if element is disabled attribute\\n\
                     - Timeout: If element doesn't appear within timeout_ms, returns timeout error\\n\
                     - No page: Returns error if browser_navigate hasn't been called first\\n\\n\
                     \\x1b[1mTroubleshooting:\\x1b[0m\\n\
                     - Use browser_screenshot to visually verify selector targets correct element\\n\
                     - Use browser_extract_text to test if your selector finds the element\\n\
                     - Increase timeout_ms if element appears slowly (SPA rendering)\\n\
                     - Check for z-index issues if element is obscured\\n\
                     - Verify selector syntax with browser DevTools console (e.g., document.querySelector(...))",
                ),
            },
        ])
    }
}
