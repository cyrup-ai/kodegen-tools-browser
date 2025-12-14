//! Browser type text tool - inputs text into form fields

use chromiumoxide::Page;
use kodegen_mcp_schema::browser::{
    BrowserTypeTextArgs, BrowserTypeOutput, BROWSER_TYPE_TEXT,
    TypeTextPrompts,
};
use kodegen_mcp_schema::{Tool, ToolExecutionContext, ToolResponse, McpError};
use std::sync::Arc;

use crate::manager::BrowserManager;
use crate::utils::validate_interaction_timeout;

/// Query the page for available input elements and format as hints
/// 
/// This helps the agent learn what selectors are actually available
/// when its guess fails.
async fn get_input_element_hints(page: &Page) -> String {
    // Try to find input elements
    let inputs = match page.find_elements("input, textarea, [contenteditable='true']").await {
        Ok(elements) => elements,
        Err(_) => return String::new(),
    };
    
    if inputs.is_empty() {
        return "No input elements found on page.".to_string();
    }
    
    let mut hints = Vec::new();
    for (i, el) in inputs.iter().take(10).enumerate() {
        // Try to get identifying attributes
        let id = el.attribute("id").await.ok().flatten();
        let name = el.attribute("name").await.ok().flatten();
        let class = el.attribute("class").await.ok().flatten();
        let placeholder = el.attribute("placeholder").await.ok().flatten();
        let input_type = el.attribute("type").await.ok().flatten();
        
        let mut selector_hints = Vec::new();
        
        if let Some(id) = id
            && !id.is_empty() {
            selector_hints.push(format!("#{}", id));
        }
        if let Some(name) = name
            && !name.is_empty() {
            selector_hints.push(format!("input[name='{}']", name));
        }
        
        // Build description
        let type_str = input_type.unwrap_or_else(|| "text".to_string());
        let placeholder_str = placeholder.map(|p| format!(" placeholder=\"{}\"", p)).unwrap_or_default();
        let class_preview = class.map(|c| {
            let first_class = c.split_whitespace().next().unwrap_or("");
            if first_class.is_empty() { String::new() } else { format!(" .{}", first_class) }
        }).unwrap_or_default();
        
        if !selector_hints.is_empty() {
            hints.push(format!(
                "  {}. [{}{}{}] → {}",
                i + 1,
                type_str,
                placeholder_str,
                class_preview,
                selector_hints.join(" or ")
            ));
        }
    }
    
    if hints.is_empty() {
        return "Input elements found but no usable selectors (missing id/name attributes).".to_string();
    }
    
    format!("Available input elements:\n{}", hints.join("\n"))
}

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

        // Get current page from manager (set by browser_navigate)
        let page = self
            .manager
            .get_current_page()
            .await
            .ok_or_else(|| {
                McpError::Other(anyhow::anyhow!(
                    "No page available. You must call browser_navigate first to load a page."
                ))
            })?;

        // Find element with polling (waits for SPAs to render)
        let timeout = validate_interaction_timeout(args.timeout_ms, 5000)?;
        let element = match crate::utils::wait_for_element(&page, &args.selector, timeout).await {
            Ok(el) => el,
            Err(e) => {
                // Element not found - get DOM hints to help the agent try a better selector
                let hints = get_input_element_hints(&page).await;
                let hint_section = if hints.is_empty() {
                    String::new()
                } else {
                    format!("\n\n{}", hints)
                };
                return Err(McpError::Other(anyhow::anyhow!(
                    "Element not found for selector '{}'. {}{}",
                    args.selector,
                    e,
                    hint_section
                )));
            }
        };

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
