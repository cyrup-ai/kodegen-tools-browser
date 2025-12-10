//! Browser navigation tool - loads URLs and waits for page ready

use kodegen_mcp_schema::browser::{
    BrowserNavigateArgs, BrowserNavigateOutput, BROWSER_NAVIGATE,
    NavigatePrompts,
};
use kodegen_mcp_schema::{Tool, ToolExecutionContext, ToolResponse, McpError};
// Removed serde_json::{json, Value} - no longer needed after conversion to typed NavigationResult
use std::sync::Arc;

use crate::manager::BrowserManager;
use crate::utils::validate_navigation_timeout;

/// Internal navigation result returned by navigate_and_capture_page()
/// 
/// This is NOT exposed via MCP schema (use BrowserNavigateOutput for that).
/// Contains additional metadata for internal logic: requested_url, redirected, message.
#[derive(Debug, Clone)]
pub(crate) struct NavigationResult {
    /// Whether navigation succeeded
    pub success: bool,
    
    /// Final URL after navigation (may differ from requested_url due to redirects)
    pub url: String,
    
    /// Originally requested URL (before any redirects)
    pub requested_url: String,
    
    /// Whether the final URL differs from requested URL
    pub redirected: bool,
    
    /// Human-readable message describing the navigation
    pub message: String,
}

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
    ) -> Result<(chromiumoxide::Page, NavigationResult), McpError> {
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
                // Log errors but continue - pages might already be closed or unresponsive
                if let Err(e) = page.close().await {
                    tracing::warn!(
                        "Failed to close page during cleanup (may already be closed): {}",
                        e
                    );
                }
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

        let result = NavigationResult {
            success: true,
            url: final_url.clone(),
            requested_url: args.url.clone(),
            redirected: final_url != args.url,
            message: format!("Navigated to {}", final_url),
        };

        // Return BOTH page and JSON (new behavior for parallel execution)
        Ok((page, result))
    }
}

impl Tool for BrowserNavigateTool {
    type Args = BrowserNavigateArgs;
    type Prompts = NavigatePrompts;

    fn name() -> &'static str {
        BROWSER_NAVIGATE
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

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<ToolResponse<BrowserNavigateOutput>, McpError> {
        // Store timeout before moving args
        let timeout_ms = args.timeout_ms.unwrap_or(30000);
        
        // Capture page handle to ensure cleanup (CRITICAL: don't use _page)
        let (page, result) = self.navigate_and_capture_page(args).await?;
        
        // Extract data from typed result
        let final_url = result.url;
        let redirected = result.redirected;
        let requested_url = result.requested_url;

        // Log navigation result for debugging
        tracing::debug!("{}", result.message);

        // Terminal summary (KODEGEN pattern: 2-line colored format)
        let summary = if redirected {
            format!(
                "\x1b[36mNavigate: {}\x1b[0m\n\
                  Redirected: {} → {} · Timeout: {}ms",
                final_url,
                requested_url,
                final_url,
                timeout_ms
            )
        } else {
            format!(
                "\x1b[36mNavigate: {}\x1b[0m\n\
                  Timeout: {}ms",
                final_url,
                timeout_ms
            )
        };

        // Build typed output
        let output = BrowserNavigateOutput {
            success: result.success,
            url: final_url,
            title: None,
            status_code: None,
        };

        // CRITICAL FIX: Close page before returning to prevent memory leak
        if let Err(e) = page.close().await {
            tracing::warn!("Failed to close navigation page: {}", e);
        }

        Ok(ToolResponse::new(summary, output))
    }
}
