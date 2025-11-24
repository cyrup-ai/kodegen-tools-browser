//! Browser navigation tool - loads URLs and waits for page ready

use kodegen_mcp_schema::browser::{BrowserNavigateArgs, BrowserNavigatePromptArgs, BROWSER_NAVIGATE};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, error::McpError};
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

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<Vec<Content>, McpError> {
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

        // Terminal summary (KODEGEN pattern: 2-line colored format)
        let summary = if redirected {
            format!(
                "\x1b[36m󰖟 Navigate: {}\x1b[0m\n\
                  Redirected: {} → {} · Timeout: {}ms",
                final_url,
                requested_url,
                final_url,
                timeout_ms
            )
        } else {
            format!(
                "\x1b[36m󰖟 Navigate: {}\x1b[0m\n\
                  Timeout: {}ms",
                final_url,
                timeout_ms
            )
        };
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
        vec![
            PromptArgument {
                name: "complexity_level".to_string(),
                title: Some("Explanation Detail Level".to_string()),
                description: Some(
                    "Choose 'basic' for essential usage patterns or 'advanced' for edge cases and troubleshooting"
                        .to_string(),
                ),
                required: Some(false),
            },
            PromptArgument {
                name: "example_focus".to_string(),
                title: Some("Example Topic Focus".to_string()),
                description: Some(
                    "Focus examples on specific features: 'general', 'selectors', 'timeouts', or 'redirects'"
                        .to_string(),
                ),
                required: Some(false),
            },
        ]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            // Exchange 1: Core purpose
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What does browser_navigate do and when should I use it?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "browser_navigate is the entry point for all browser automation. It loads a URL in a headless \
                     Chromium browser and waits for the page to fully load.\n\n\
                     USE navigate when you need to:\n\
                     - Load a website to interact with it (click buttons, type text)\n\
                     - Capture the current page state (screenshot, extract text)\n\
                     - Access a page before performing other browser operations\n\n\
                     DO NOT use navigate for:\n\
                     - Quick web searches → use browser_web_search instead\n\
                     - Deep website research with multiple page visits → use browser_research instead\n\
                     - Just checking if a URL exists → use browser_web_search\n\n\
                     navigate MUST be called first in any automation flow. Subsequent tools (click, type, screenshot) \
                     operate on the page you navigated to.",
                ),
            },
            
            // Exchange 2: Basic usage
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("Show me a basic navigation example"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Here's the simplest example:\n\n\
                     browser_navigate({\"url\": \"https://www.example.com\"})\n\n\
                     This loads the page and waits for:\n\
                     - Initial page load to complete\n\
                     - Document ready state\n\
                     - All synchronous JavaScript execution\n\n\
                     The response includes:\n\
                     - url: The final URL (may differ from requested URL due to redirects)\n\
                     - requested_url: The URL you asked to navigate to\n\
                     - redirected: Boolean indicating if a redirect occurred\n\
                     - success: Always true if no error was thrown\n\n\
                     After successful navigation, the page stays in memory and is available for:\n\
                     - browser_click() to interact with elements\n\
                     - browser_type_text() to enter form data\n\
                     - browser_screenshot() to capture the page\n\
                     - browser_extract_text() to read page content\n\
                     - browser_scroll() to move around the page",
                ),
            },

            // Exchange 3: URL requirements
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("What are the requirements for the URL parameter?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "URL Requirements:\n\n\
                     1. MUST start with http:// or https://\n\
                        ✓ browser_navigate({\"url\": \"https://example.com\"})\n\
                        ✓ browser_navigate({\"url\": \"http://localhost:3000\"})\n\
                        ✗ browser_navigate({\"url\": \"example.com\"}) - ERROR: missing protocol\n\
                        ✗ browser_navigate({\"url\": \"ftp://example.com\"}) - ERROR: unsupported protocol\n\n\
                     2. MUST be a valid, complete URL\n\
                        ✓ \"https://api.example.com/endpoint?param=value\"\n\
                        ✗ \"/path/to/page\" - relative URLs not supported\n\n\
                     3. MUST be accessible from the browser's network\n\
                        - Public URLs work reliably\n\
                        - Private/internal URLs may fail (firewall, VPN, authentication)\n\
                        - Localhost/127.0.0.1 works only if you control the local server\n\n\
                     Before calling navigate, validate the URL:\n\
                     - Ensure it's complete (has scheme, host, valid format)\n\
                     - Verify it's actually accessible (try in a real browser first)\n\
                     - Check if the site is known to block headless browsers",
                ),
            },

            // Exchange 4: Timeout handling
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "How does the timeout_ms parameter work and when should I increase it?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Timeout Behavior:\n\n\
                     timeout_ms controls how long navigate waits (in milliseconds) for the page to load:\n\
                     - Default: 30000ms (30 seconds)\n\
                     - Range: Any positive integer in milliseconds\n\n\
                     Examples:\n\
                     browser_navigate({\n\
                       \"url\": \"https://example.com\",\n\
                       \"timeout_ms\": 10000  // 10 second timeout\n\
                     })\n\n\
                     INCREASE timeout for:\n\
                     - Heavy JavaScript frameworks (React, Vue) that take time to initialize\n\
                     - Slow networks or servers\n\
                     - Pages with many images/resources: timeout_ms: 60000\n\
                     - Complex SPAs that load data: timeout_ms: 45000\n\
                     - Sites with slow third-party trackers: timeout_ms: 50000\n\n\
                     DECREASE timeout for:\n\
                     - Fast, static websites: timeout_ms: 10000\n\
                     - APIs that respond quickly: timeout_ms: 5000\n\
                     - When you want to fail fast and retry\n\n\
                     TIMEOUT ERROR occurs when:\n\
                     - Page takes longer than timeout_ms to load\n\
                     - Browser is blocked by security (anti-bot, Cloudflare, etc.)\n\
                     - Network connection is broken\n\n\
                     If you get timeout errors:\n\
                     1. Try increasing timeout_ms (double the current value)\n\
                     2. Verify the URL is accessible (test manually)\n\
                     3. Some sites block headless browsers - no solution here\n\
                     4. Check network connectivity",
                ),
            },

            // Exchange 5: Selector waiting
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What is wait_for_selector and when should I use it?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "wait_for_selector is an OPTIONAL parameter that makes navigate wait for a specific element to appear:\n\n\
                     browser_navigate({\n\
                       \"url\": \"https://example.com\",\n\
                       \"wait_for_selector\": \"#main-content\"\n\
                     })\n\n\
                     This is useful for:\n\
                     - Single-Page Applications (SPAs) that render content after load\n\
                     - Pages with JavaScript frameworks (React, Angular, Vue)\n\
                     - Waiting for dynamic content to appear before proceeding\n\n\
                     CSS Selector Format (same as browser_click):\n\
                     - ID selector: wait_for_selector: \"#header\"\n\
                     - Class selector: wait_for_selector: \".main-container\"\n\
                     - Attribute selector: wait_for_selector: \"[data-testid='results']\"\n\
                     - Type selector: wait_for_selector: \"button.submit\"\n\
                     - Pseudo-selector: wait_for_selector: \"div:nth-child(1)\"\n\
                     - Complex: wait_for_selector: \"main > article.content\"\n\n\
                     COMMON PATTERN: Wait for body as fallback\n\
                     browser_navigate({\n\
                       \"url\": \"https://example.com\",\n\
                       \"wait_for_selector\": \"body\"\n\
                     })\n\
                     This ensures the page DOM is available.\n\n\
                     When selector NOT found within timeout:\n\
                     - Error is returned explaining selector not found\n\
                     - Try using a different selector\n\
                     - Increase timeout_ms if JavaScript is still initializing\n\
                     - Verify selector matches an element on the page (test in DevTools)",
                ),
            },

            // Exchange 6: Redirect handling
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How does navigate handle redirects?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Redirect Handling (Automatic):\n\n\
                     browser_navigate automatically follows HTTP redirects:\n\n\
                     browser_navigate({\"url\": \"http://example.com\"})\n\
                     // If example.com redirects to https://www.example.com\n\
                     // navigate automatically follows and lands on www.example.com\n\n\
                     Response indicates if redirect occurred:\n\n\
                     {\n\
                       \"success\": true,\n\
                       \"url\": \"https://www.example.com\",        // FINAL URL\n\
                       \"requested_url\": \"http://example.com\",   // ORIGINAL URL\n\
                       \"redirected\": true,                        // Redirect occurred\n\
                       \"message\": \"Navigated to https://www.example.com\"\n\
                     }\n\n\
                     Use the 'redirected' and 'url' fields to:\n\
                     - Verify you ended up where expected\n\
                     - Detect unexpected redirects (spam, hijacking)\n\
                     - Track canonical URLs\n\n\
                     Common redirect chains:\n\
                     http://example.com → https://example.com → https://www.example.com\n\
                     navigate handles the entire chain and returns the final destination.\n\n\
                     GOTCHA: Some sites redirect based on User-Agent or headers\n\
                     - Headless browsers may get different redirects than browsers\n\
                     - This is normal and usually expected",
                ),
            },

            // Exchange 7: Integration workflow
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "Show me a complete workflow using navigate with other browser tools",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Typical Browser Automation Workflow:\n\n\
                     STEP 1: Navigate to the page\n\
                     browser_navigate({\n\
                       \"url\": \"https://example.com/search\",\n\
                       \"wait_for_selector\": \".search-form\"\n\
                     })\n\n\
                     STEP 2: Capture initial state (optional)\n\
                     browser_screenshot({})\n\n\
                     STEP 3: Interact with the page\n\
                     browser_type_text({\n\
                       \"selector\": \"input[name='q']\",\n\
                       \"text\": \"rust programming\"\n\
                     })\n\
                     browser_click({\"selector\": \"button.search\"})\n\n\
                     STEP 4: Wait for results and extract\n\
                     browser_navigate({\n\
                       \"url\": \"<current URL>\",  // Click may have navigated\n\
                       \"wait_for_selector\": \".results\"\n\
                     })\n\
                     browser_extract_text({\"selector\": \".results\"})\n\n\
                     STEP 5: Scroll for more content\n\
                     browser_scroll({\"direction\": \"down\", \"amount\": 3})\n\n\
                     KEY POINTS:\n\
                     - navigate ALWAYS comes first or after major page changes\n\
                     - After navigate, page state persists for subsequent tools\n\
                     - Each click/interaction may require a new navigate if page changes\n\
                     - Use wait_for_selector to ensure content is ready before proceeding",
                ),
            },

            // Exchange 8: Common errors and solutions
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What are the most common errors and how do I fix them?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Common Errors and Solutions:\n\n\
                     ERROR 1: \"URL must start with http:// or https://\"\n\
                     Cause: Invalid URL format\n\
                     Solution: Ensure URL has correct protocol\n\
                     ✗ \"google.com\" → ✓ \"https://google.com\"\n\
                     ✗ \"localhost:3000\" → ✓ \"http://localhost:3000\"\n\n\
                     ERROR 2: \"Navigation timeout after XXXms\"\n\
                     Causes:\n\
                     - Site is slow or blocking headless browsers\n\
                     - Network connectivity issues\n\
                     - Page takes longer than timeout to load\n\
                     Solutions:\n\
                     1. Increase timeout_ms: timeout_ms: 60000\n\
                     2. Verify URL works in a regular browser\n\
                     3. Check network connectivity\n\
                     4. Some sites actively block headless browsers (no solution)\n\n\
                     ERROR 3: \"Browser not available\"\n\
                     Cause: Browser process crashed or wasn't launched\n\
                     Solution: This is internal - report if it happens\n\n\
                     ERROR 4: Selector not found error\n\
                     Causes:\n\
                     - wait_for_selector doesn't match any element\n\
                     - Selector syntax is incorrect\n\
                     - Element hasn't rendered yet\n\
                     Solutions:\n\
                     1. Check selector syntax (use CSS selector format)\n\
                     2. Increase timeout_ms if page is slow\n\
                     3. Use browser DevTools to verify selector exists\n\
                     4. Try simpler selector: wait_for_selector: \"body\"\n\n\
                     ERROR 5: \"Navigation failed for URL\"\n\
                     Causes:\n\
                     - URL returns HTTP error (404, 500, etc.)\n\
                     - Site certificate is invalid\n\
                     - Network unreachable\n\
                     Solutions:\n\
                     1. Verify URL is correct\n\
                     2. Try URL in a regular browser\n\
                     3. Check if site requires authentication\n\
                     4. For HTTPS errors, URL may have SSL issues",
                ),
            },

            // Exchange 9: Best practices
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("What are best practices for reliable navigation?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Best Practices for Reliable Navigation:\n\n\
                     1. VALIDATE URLS BEFORE NAVIGATION\n\
                        - Use browser_web_search to validate accessibility first\n\
                        - Ensure URLs are complete and well-formed\n\
                        - Test problematic URLs manually first\n\n\
                     2. SET APPROPRIATE TIMEOUTS\n\
                        - Dynamic sites (React, Angular): 45000ms or more\n\
                        - Regular websites: 30000ms (default)\n\
                        - Fast/static sites: 10000ms\n\
                        - APIs: 5000ms\n\n\
                     3. USE wait_for_selector FOR ASYNC PAGES\n\
                        - Single-Page Applications: wait for main content\n\
                        - JavaScript-heavy sites: wait for key elements\n\
                        - Fallback: wait_for_selector: \"body\"\n\n\
                     4. HANDLE REDIRECTS\n\
                        - Check the 'redirected' field in response\n\
                        - Verify you ended up at expected URL\n\
                        - Be aware some sites redirect based on User-Agent\n\n\
                     5. ORGANIZE YOUR WORKFLOW\n\
                        - navigate → screenshot → [click/type] → navigate (if changed) → extract\n\
                        - Don't assume page persists across major changes\n\
                        - Re-navigate after clicking navigation links\n\n\
                     6. ERROR HANDLING\n\
                        - Timeout errors: increase timeout_ms and retry\n\
                        - Selector not found: use simpler selector or increase timeout\n\
                        - Connection errors: verify URL and network\n\n\
                     7. PERFORMANCE\n\
                        - Don't set unnecessarily high timeouts (wastes time)\n\
                        - Reuse single page for multiple operations when possible\n\
                        - For multiple unrelated pages, navigate freshly each time",
                ),
            },

            // Exchange 10: Advanced scenarios
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What about advanced scenarios like authentication or multiple pages?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Advanced Scenarios:\n\n\
                     SCENARIO 1: Login-Protected Sites\n\
                     browser_navigate({\"url\": \"https://secure-site.com/login\"})\n\
                     browser_type_text({\"selector\": \"input[name='username']\", \"text\": \"user@example.com\"})\n\
                     browser_type_text({\"selector\": \"input[name='password']\", \"text\": \"password\"})\n\
                     browser_click({\"selector\": \"button.login\", \"wait_for_navigation\": true})\n\
                     // After login, navigate to target page\n\
                     browser_navigate({\"url\": \"https://secure-site.com/dashboard\"})\n\n\
                     NOTE: Credentials should be handled securely, never hardcoded in prompts\n\n\
                     SCENARIO 2: Multiple Sequential Pages\n\
                     // Page 1\n\
                     browser_navigate({\"url\": \"https://site.com/page1\"})\n\
                     browser_click({\"selector\": \"a.next\", \"wait_for_navigation\": true})\n\
                     \n\
                     // Page 2 (URL changed from click)\n\
                     browser_navigate({\"url\": \"<new URL from click>\"})  // Navigate to updated URL\n\
                     browser_extract_text({\"selector\": \".content\"})\n\n\
                     SCENARIO 3: JavaScript-Heavy Sites (React, Vue, Angular)\n\
                     browser_navigate({\n\
                       \"url\": \"https://spa-site.com\",\n\
                       \"timeout_ms\": 45000,\n\
                       \"wait_for_selector\": \".app-container\"  // Main app component\n\
                     })\n\n\
                     SCENARIO 4: Sites with Anti-Bot Detection\n\
                     Some sites actively detect and block headless browsers:\n\
                     - Headless Chrome detection patterns\n\
                     - Cloudflare challenges\n\
                     - reCAPTCHA\n\
                     \n\
                     These are difficult/impossible to handle automatically.\n\
                     For critical sites: use browser_research which has anti-detection built-in\n\n\
                     SCENARIO 5: Dynamic Content Loading (Infinite Scroll)\n\
                     browser_navigate({\"url\": \"https://infinite-scroll-site.com\"})\n\
                     browser_scroll({\"direction\": \"down\", \"amount\": 5})\n\
                     // More content loads\n\
                     browser_extract_text({\"selector\": \".feed\"})\n\
                     // Can repeat scroll/extract cycle\n\n\
                     For Complex Scenarios → Consider browser_research\n\
                     browser_research is designed for complex multi-step scenarios, handling:\n\
                     - Navigation chains across multiple pages\n\
                     - Anti-bot detection\n\
                     - Complex user flows\n\
                     - Detailed information extraction",
                ),
            },
        ])
    }
}
