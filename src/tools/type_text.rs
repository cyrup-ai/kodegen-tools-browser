//! Browser type text tool - inputs text into form fields

use kodegen_mcp_schema::browser::{
    BrowserTypeTextArgs, BrowserTypeOutput, BROWSER_TYPE_TEXT,
    TypeTextPrompts,
};
use kodegen_mcp_schema::{Tool, ToolExecutionContext, ToolResponse, McpError};
use std::sync::Arc;

use crate::manager::BrowserManager;
use crate::utils::validate_interaction_timeout;

#[derive(Clone)]
pub struct BrowserTypeTextTool {
    manager: Arc<BrowserManager>,
}

impl BrowserTypeTextTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

impl Tool for BrowserTypeTextTool {
    type Args = BrowserTypeTextArgs;
    type Prompts = TypeTextPrompts;

    fn name() -> &'static str {
        BROWSER_TYPE_TEXT
    }

    fn description() -> &'static str {
        "Type text into an input element using a CSS selector.\\n\\n\
         Automatically focuses element and clears existing text by default.\\n\\n\
         Example: browser_type_text({\\\"selector\\\": \\\"#email\\\", \\\"text\\\": \\\"user@test.local\\\"})\\n\
         Example: browser_type_text({\\\"selector\\\": \\\"#search\\\", \\\"text\\\": \\\"query\\\", \\\"clear\\\": false})"
    }

    fn read_only() -> bool {
        false // Typing changes page state
    }

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<ToolResponse<BrowserTypeOutput>, McpError> {
        // Validate selector
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

        // Click element to focus (bypass IntersectionObserver hang)
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
                "Click to focus failed for selector '{}'. \
                 Possible causes: (1) Element is obscured by another element, \
                 (2) Element is disabled or not focusable, \
                 (3) Page changed after finding element. \
                 Error: {}",
                args.selector,
                e
            ))
        })?;

        // Clear existing text if requested
        if args.clear {
            element
                .call_js_fn("function() { this.value = ''; }", false)
                .await
                .map_err(|e| {
                    McpError::Other(anyhow::anyhow!(
                        "Failed to clear field for selector '{}'. \
                         Possible causes: (1) Element is not an input/textarea field, \
                         (2) Field is read-only or disabled, \
                         (3) JavaScript execution was blocked. \
                         Error: {}",
                        args.selector,
                        e
                    ))
                })?;
        }

        // Type text
        element.type_str(&args.text).await.map_err(|e| {
            McpError::Other(anyhow::anyhow!(
                "Type text failed for selector '{}'. \
                 Possible causes: (1) Element lost focus during typing, \
                 (2) Element is not a text input field, \
                 (3) Field has input restrictions or validation. \
                 Error: {}",
                args.selector,
                e
            ))
        })?;

        let text_len = args.text.len();

        // Terminal summary
        let summary = format!(
            "\x1b[33m\u{f11d} Type Text: {}\x1b[0m\n\
             \u{f129} Element: {} · Characters: {}",
            args.selector,
            args.selector,
            text_len
        );

        // Build typed output
        let output = BrowserTypeOutput {
            success: true,
            selector: args.selector,
            text_length: text_len,
            message: format!("Typed {} characters", text_len),
        };

        Ok(ToolResponse::new(summary, output))
    }
}
