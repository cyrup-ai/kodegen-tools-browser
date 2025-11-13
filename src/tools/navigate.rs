//! Browser navigation tool - loads URLs and waits for page ready

use kodegen_mcp_schema::browser::{BrowserNavigateArgs, BrowserNavigatePromptArgs};
use kodegen_mcp_tool::{Tool, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::manager::BrowserManager;
use crate::utils::validate_navigation_timeout;

#[derive(Clone)]
pub struct BrowserNavigateTool {
    manager: Arc<BrowserManager>,
}

impl BrowserNavigateTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }

    /// Internal method that returns both Page handle and result JSON
    /// 
    /// Used by deep_research to capture specific page in parallel execution.
    /// External MCP callers use execute() which discards Page handle.
    pub(crate) async fn navigate_and_capture_page(
        &self,
        args: BrowserNavigateArgs,
    ) -> Result<(chromiumoxide::Page, Value), McpError> {
        // Validate URL protocol
        if !args.url.starts_with("http://") && !args.url.starts_with("https://") {
            return Err(McpError::invalid_arguments(
                "URL must start with http:// or https://",
            ));
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

        // Close all existing pages to enforce single-page model
        // Prevents non-deterministic page selection in get_current_page()
        if let Ok(existing_pages) = wrapper.browser().pages().await {
            for page in existing_pages {
                // Ignore errors - pages might already be closed or unresponsive
                let _ = page.close().await;
            }
        }

        // Create new blank page (now guaranteed to be the ONLY page)
        let page = crate::browser::create_blank_page(wrapper)
            .await
            .map_err(McpError::Other)?;

        // Navigate to URL
        let timeout = validate_navigation_timeout(args.timeout_ms, 30000)?;
        tokio::time::timeout(timeout, page.goto(&args.url))
            .await
            .map_err(|_| {
                McpError::Other(anyhow::anyhow!(
                    "Navigation timeout after {}ms for URL: {}. \
                     Try: (1) Increase timeout_ms parameter (default: 30000), \
                     (2) Verify URL is accessible in a browser, \
                     (3) Check if site blocks headless browsers.",
                    timeout.as_millis(),
                    args.url
                ))
            })?
            .map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Navigation failed for URL: {}. \
                     Check: (1) URL is correctly formatted, \
                     (2) Network connectivity, \
                     (3) URL returns a valid HTTP response. \
                     Error: {}",
                    args.url,
                    e
                ))
            })?;
        
        // Wait for page lifecycle to complete
        // Pattern from web_search/search.rs - wait_for_navigation ensures page is fully loaded
        page.wait_for_navigation()
            .await
            .map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Failed to wait for page load completion: {}",
                    e
                ))
            })?;

        // Wait for selector if specified
        if let Some(selector) = &args.wait_for_selector {
            crate::utils::wait_for_element(&page, selector, timeout).await?;
        }

        // Get final URL (may differ from requested due to redirects)
        let final_url = page
            .url()
            .await
            .map_err(|e| McpError::Other(anyhow::anyhow!("Failed to get URL: {}", e)))?
            .unwrap_or_else(|| args.url.clone());

        let result = json!({
            "success": true,
            "url": final_url,
            "requested_url": args.url,
            "redirected": final_url != args.url,
            "message": format!("Navigated to {}", final_url)
        });

        // Return BOTH page and JSON (new behavior for parallel execution)
        Ok((page, result))
    }
}

impl Tool for BrowserNavigateTool {
    type Args = BrowserNavigateArgs;
    type PromptArgs = BrowserNavigatePromptArgs;

    fn name() -> &'static str {
        "browser_navigate"
    }

    fn description() -> &'static str {
        "Navigate to a URL in the browser. Opens the page and waits for load completion.\\n\\n\
         Returns current URL after navigation (may differ from requested URL due to redirects).\\n\\n\
         Example: browser_navigate({\\\"url\\\": \\\"https://www.rust-lang.org\\\"})\\n\
         With selector wait: browser_navigate({\\\"url\\\": \\\"https://httpbin.org/html\\\", \\\"wait_for_selector\\\": \\\"body\\\"})"
    }

    fn read_only() -> bool {
        false // Navigation changes browser state
    }

    fn open_world() -> bool {
        true // Accesses external URLs
    }

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
        // Store args values before moving into navigate_and_capture_page
        let timeout_ms = args.timeout_ms.unwrap_or(30000);
        let requested_url = args.url.clone();
        
        // Delegate to internal method and discard Page handle
        let (_page, result) = self.navigate_and_capture_page(args).await?;
        
        // Extract data from result JSON
        let final_url = result.get("url")
            .and_then(|v| v.as_str())
            .unwrap_or(&requested_url);
        let redirected = result.get("redirected")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        
        let mut contents = Vec::new();

        // Terminal summary
        let redirect_info = if redirected {
            format!("\n  Redirected: {} → {}", requested_url, final_url)
        } else {
            String::new()
        };

        let summary = format!(
            "✓ Navigation complete\n\n\
             URL: {}{}\n\
             Timeout: {}ms",
            final_url,
            redirect_info,
            timeout_ms
        );
        contents.push(Content::text(summary));

        // JSON metadata (preserve all original fields)
        let metadata = json!({
            "success": true,
            "url": final_url,
            "requested_url": requested_url,
            "redirected": redirected,
            "timeout_ms": timeout_ms,
            "message": format!("Navigated to {}", final_url)
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
                content: PromptMessageContent::text("How do I navigate to a website?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use browser_navigate with a url parameter. Example: {\\\"url\\\": \\\"https://www.rust-lang.org\\\"}\\n\\n\
                     You can also wait for elements: {\\\"url\\\": \\\"https://httpbin.org/html\\\", \\\"wait_for_selector\\\": \\\"body\\\"}\\n\
                     Increase timeout if needed: {\\\"url\\\": \\\"https://slow-site.com\\\", \\\"timeout_ms\\\": 60000}",
                ),
            },
        ])
    }
}
