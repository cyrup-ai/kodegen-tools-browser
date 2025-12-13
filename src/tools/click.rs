//! Browser click tool - clicks elements by CSS selector

use chromiumoxide::Page;
use kodegen_mcp_schema::browser::{
    BrowserClickArgs, BrowserClickOutput, BROWSER_CLICK,
    ClickPrompts,
};
use kodegen_mcp_schema::{Tool, ToolExecutionContext, ToolResponse, McpError};
use std::sync::Arc;

use crate::manager::BrowserManager;
use crate::utils::validate_interaction_timeout;

/// Query the page for clickable elements and format as hints
/// 
/// This helps the agent learn what selectors are actually available
/// when its guess fails.
async fn get_clickable_element_hints(page: &Page) -> String {
    // Try to find clickable elements
    let clickables = match page.find_elements("button, a, [role='button'], input[type='submit'], input[type='button']").await {
        Ok(elements) => elements,
        Err(_) => return String::new(),
    };
    
    if clickables.is_empty() {
        return "No clickable elements found on page.".to_string();
    }
    
    let mut hints = Vec::new();
    for (i, el) in clickables.iter().take(15).enumerate() {
        // Try to get identifying attributes
        let id = el.attribute("id").await.ok().flatten();
        let name = el.attribute("name").await.ok().flatten();
        let class = el.attribute("class").await.ok().flatten();
        let text = el.inner_text().await.ok().flatten();
        let href = el.attribute("href").await.ok().flatten();
        let role = el.attribute("role").await.ok().flatten();
        // Get tag name via JavaScript since chromiumoxide Element doesn't expose it directly
        let tag: Option<String> = el.call_js_fn("function() { return this.tagName; }", false)
            .await
            .ok()
            .and_then(|v| v.result.value)
            .and_then(|val| val.as_str().map(|s| s.to_lowercase()));
        
        let mut selector_hints = Vec::new();
        
        if let Some(id) = &id
            && !id.is_empty() {
            selector_hints.push(format!("#{}", id));
        }
        if let Some(name) = &name
            && !name.is_empty() {
            selector_hints.push(format!("[name='{}']", name));
        }
        
        // Build description
        let tag_str = tag.unwrap_or_else(|| "element".to_string());
        let text_preview = text.map(|t| {
            let trimmed = t.trim();
            if trimmed.len() > 20 {
                format!(" \"{}...\"", &trimmed[..20])
            } else if !trimmed.is_empty() {
                format!(" \"{}\"", trimmed)
            } else {
                String::new()
            }
        }).unwrap_or_default();
        let href_preview = href.map(|h| format!(" href=\"{}\"", if h.len() > 30 { &h[..30] } else { &h })).unwrap_or_default();
        let role_str = role.map(|r| format!(" role={}", r)).unwrap_or_default();
        let class_preview = class.map(|c| {
            let first_class = c.split_whitespace().next().unwrap_or("");
            if first_class.is_empty() { String::new() } else { format!(" .{}", first_class) }
        }).unwrap_or_default();
        
        if !selector_hints.is_empty() {
            hints.push(format!(
                "  {}. <{}{}{}{}{}> → {}",
                i + 1,
                tag_str,
                text_preview,
                href_preview,
                role_str,
                class_preview,
                selector_hints.join(" or ")
            ));
        }
    }
    
    if hints.is_empty() {
        return "Clickable elements found but no usable selectors (missing id/name attributes).".to_string();
    }
    
    format!("Available clickable elements:\n{}", hints.join("\n"))
}

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
    type Prompts = ClickPrompts;

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

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<ToolResponse<BrowserClickOutput>, McpError> {
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
        let element = match crate::utils::wait_for_element(&page, &args.selector, timeout).await {
            Ok(el) => el,
            Err(e) => {
                // Element not found - get DOM hints to help the agent try a better selector
                let hints = get_clickable_element_hints(&page).await;
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

        // Terminal summary
        let summary = format!(
            "\x1b[33m  Click: {}\x1b[0m\n \
              Element: {} · Action: clicked",
            args.selector,
            args.selector
        );

        // Build typed output
        let output = BrowserClickOutput {
            success: true,
            selector: args.selector,
            message: "Element clicked successfully".to_string(),
        };

        Ok(ToolResponse::new(summary, output))
    }
}
