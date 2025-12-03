//! Browser scroll tool - scrolls page or to specific element

use chromiumoxide_cdp::cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams};
use kodegen_mcp_schema::browser::{
    BrowserScrollArgs, BrowserScrollOutput, BrowserScrollPromptArgs, BROWSER_SCROLL,
};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, ToolResponse, error::McpError};
use rmcp::model::{PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;
use std::sync::Arc;
use tracing::warn;

use crate::manager::BrowserManager;

#[derive(Clone)]
pub struct BrowserScrollTool {
    manager: Arc<BrowserManager>,
}

impl BrowserScrollTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

impl Tool for BrowserScrollTool {
    type Args = BrowserScrollArgs;
    type PromptArgs = BrowserScrollPromptArgs;

    fn name() -> &'static str {
        BROWSER_SCROLL
    }

    fn description() -> &'static str {
        "Scroll the page by amount or to a specific element.\\n\\n\
         Examples:\\n\
         - browser_scroll({\"y\": 500}) - Scroll down 500px\\n\
         - browser_scroll({\"selector\": \"#footer\"}) - Scroll to element"
    }

    fn read_only() -> bool {
        false // Scrolling changes viewport state
    }

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<ToolResponse<BrowserScrollOutput>, McpError> {
        // Get browser instance
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

        // Perform scroll
        if let Some(selector) = &args.selector {
            // Find element first (validates existence)
            let element = page.find_element(selector).await.map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Element not found for selector '{}'. \
                     Verify: (1) Selector syntax is valid CSS, \
                     (2) Element exists on current page, \
                     (3) Element is not in an iframe (unsupported). \
                     Error: {}",
                    selector,
                    e
                ))
            })?;

            // Use chromiumoxide's scroll_into_view() (has IntersectionObserver check)
            element.scroll_into_view().await.map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Scroll to element failed. \
                     Possible causes: (1) Element is not scrollable or not in viewport, \
                     (2) Page structure prevents scrolling, \
                     (3) Element is detached from DOM. \
                     Error: {}",
                    e
                ))
            })?;

            // Terminal summary
            let summary = format!(
                "\x1b[33m ↻ Scroll: to element\x1b[0m\n\
                  Selector: {} · Action: scroll_to_element",
                selector
            );

            // Build typed output
            let output = BrowserScrollOutput {
                success: true,
                direction: "to_element".to_string(),
                amount: 0,
                message: format!("Scrolled to element: {}", selector),
            };

            Ok(ToolResponse::new(summary, output))
        } else {
            // Scroll by amount
            // Validate scroll amounts (defense-in-depth)
            // Agent validates, but tool should also validate since it's a public MCP tool
            let x = args.x.unwrap_or(0).clamp(-10_000, 10_000);
            let y = args.y.unwrap_or(0).clamp(-10_000, 10_000);

            // Warn if attempting to scroll zero pixels
            if x == 0 && y == 0 {
                warn!("Scroll called with x=0, y=0 (no-op)");
            }

            // Safe: parameterized evaluation prevents injection
            let call = CallFunctionOnParams::builder()
                .function_declaration("(x, y) => window.scrollBy(x, y)")
                .argument(CallArgument::builder().value(json!(x)).build())
                .argument(CallArgument::builder().value(json!(y)).build())
                .build()
                .map_err(|e| {
                    McpError::Other(anyhow::anyhow!("Failed to build scroll params: {}", e))
                })?;

            page.evaluate_function(call).await.map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Scroll by amount failed. \
                     Possible causes: (1) Page does not support scrolling, \
                     (2) Scroll amount exceeds page boundaries, \
                     (3) JavaScript execution was blocked. \
                     Error: {}",
                    e
                ))
            })?;

            // Compute direction from x/y values
            let direction = match (x, y) {
                (0, 0) => "none",
                (0, y_val) if y_val > 0 => "down",
                (0, _) => "up",
                (x_val, 0) if x_val > 0 => "right",
                (_, 0) => "left",
                (x_val, y_val) if x_val > 0 && y_val > 0 => "down-right",
                (x_val, y_val) if x_val < 0 && y_val > 0 => "down-left",
                (x_val, y_val) if x_val > 0 && y_val < 0 => "up-right",
                _ => "up-left",
            };

            let total_distance = x.abs() + y.abs();

            // Terminal summary
            let summary = format!(
                "\x1b[33m ↻ Scroll: {}\x1b[0m\n\
                  Direction: {} · Distance: {}px",
                direction, direction, total_distance
            );

            // Build typed output
            let output = BrowserScrollOutput {
                success: true,
                direction: direction.to_string(),
                amount: total_distance,
                message: format!("Scrolled by x={}, y={}", x, y),
            };

            Ok(ToolResponse::new(summary, output))
        }
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![PromptArgument {
            name: "scenario".to_string(),
            title: None,
            description: Some(
                "Optional use case scenario: 'pixel' (scroll by x/y amounts), \
                 'selector' (scroll to element), or 'both' (comprehensive overview)"
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
                    "How do I use browser_scroll to navigate pages effectively?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The browser_scroll tool supports two complementary scrolling approaches:\n\n\
                     **Mode 1: Scroll by Pixel Amounts**\n\
                     For precise, distance-based scrolling:\n\
                     ```json\n\
                     browser_scroll({\"y\": 500})              // Scroll down 500px\n\
                     browser_scroll({\"y\": -300})             // Scroll up 300px\n\
                     browser_scroll({\"x\": 200, \"y\": 400})  // Scroll right+down\n\
                     ```\n\n\
                     Pixel amounts are automatically clamped to ±10,000px for safety.\n\n\
                     **Mode 2: Scroll to Element**\n\
                     For semantic, target-based scrolling:\n\
                     ```json\n\
                     browser_scroll({\"selector\": \"#footer\"})           // Scroll to element by ID\n\
                     browser_scroll({\"selector\": \".pricing-table\"})   // Scroll to class\n\
                     browser_scroll({\"selector\": \"[data-section='contact']\"})\n\
                     ```\n\n\
                     **When to Use Each Mode:**\n\
                     - Use pixel scrolling for: pagination, viewport repositioning, smooth continuous movement\n\
                     - Use selector scrolling for: accessing specific UI components, form fields, content sections\n\n\
                     **Important Considerations:**\n\
                     1. Call browser_navigate() first - the page must be loaded before scrolling\n\
                     2. Selector mode validates element existence before scrolling (fails if not found)\n\
                     3. Pixel mode may fail if page doesn't support scrolling (e.g., overflow:hidden on body)\n\
                     4. Use scroll_into_view() behavior - respects CSS scroll-behavior properties\n\
                     5. Iframes are NOT supported - can only scroll main document\n\n\
                     **Common Patterns:**\n\
                     - Multiple scrolls: Combine with screenshot() between scrolls to inspect content\n\
                     - Scrolling to footer: browser_scroll({\"selector\": \"footer\"})\n\
                     - Exhaustive reading: Scroll down in 500px increments with extract_text() at each step\n\
                     - Smart targeting: Use data-* attributes for reliable selectors across DOM changes",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What happens if my selector doesn't exist or scrolling fails?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Error handling and recovery:\n\n\
                     **Selector Mode Errors:**\n\
                     - If element not found: Tool returns detailed error with CSS selector syntax hints\n\
                     - Verify: (1) CSS selector is valid, (2) element exists on current page, (3) element is not in iframe\n\
                     - Recovery: Use extract_text() or screenshot() to inspect page, find correct selector\n\n\
                     **Pixel Mode Errors:**\n\
                     - If scroll fails: Page may not support scrolling (check CSS overflow properties)\n\
                     - Element may be detached from DOM if page modified\n\
                     - JavaScript execution was blocked (rare)\n\
                     - Recovery: Call browser_navigate() to reload, verify page is stable before scrolling\n\n\
                     **Best Practices:**\n\
                     - Always check page state with screenshot() before and after scrolling\n\
                     - For selector scrolling, prefer unique IDs or semantic data-* attributes\n\
                     - For pixel scrolling, start with small amounts (100-300px) then adjust\n\
                     - Test selectors with extract_text(selector) before scroll operations\n\
                     - Chain operations: navigate → screenshot → scroll → screenshot to verify position",
                ),
            },
        ])
    }
}
