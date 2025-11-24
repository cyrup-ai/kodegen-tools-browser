//! Browser extract text tool - gets page or element text content

use kodegen_mcp_schema::browser::{BrowserExtractTextArgs, BrowserExtractTextPromptArgs, BROWSER_EXTRACT_TEXT};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;
use std::sync::Arc;

use crate::manager::BrowserManager;

#[derive(Clone)]
pub struct BrowserExtractTextTool {
    manager: Arc<BrowserManager>,
}

impl BrowserExtractTextTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

impl Tool for BrowserExtractTextTool {
    type Args = BrowserExtractTextArgs;
    type PromptArgs = BrowserExtractTextPromptArgs;

    fn name() -> &'static str {
        BROWSER_EXTRACT_TEXT
    }

    fn description() -> &'static str {
        "Extract text content from the page or specific element.\\n\\n\
         Returns the text content for AI agent analysis.\\n\\n\
         Example: browser_extract_text({}) - Full page text\\n\
         Example: browser_extract_text({\\\"selector\\\": \\\"#content\\\"}) - Specific element\\n\
         Example: browser_extract_text({\\\"selector\\\": \\\"article.post\\\"}) - By class"
    }

    fn read_only() -> bool {
        true // Extraction doesn't modify page
    }

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<Vec<Content>, McpError> {
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

        // Extract text based on selector
        let text = if let Some(selector) = &args.selector {
            // Extract from specific element
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

            // Get element's inner text
            element
                .inner_text()
                .await
                .map_err(|e| {
                    McpError::Other(anyhow::anyhow!(
                        "Failed to get text from selector '{}'. \
                         Possible causes: (1) Element has no text content, \
                         (2) Element is not rendered or detached from DOM, \
                         (3) Browser is in an invalid state. \
                         Error: {}",
                        selector,
                        e
                    ))
                })?
                .unwrap_or_default()
        } else {
            // Extract from entire page using JavaScript
            // Try immediate extraction first (works for SSR sites like LinkedIn)
            let eval_result = page
                .evaluate("document.body.innerText")
                .await
                .map_err(|e| {
                    McpError::Other(anyhow::anyhow!(
                        "Failed to extract page text. \
                         Possible causes: (1) Page has not fully loaded, \
                         (2) JavaScript execution was blocked, \
                         (3) Page body is empty or inaccessible. \
                         Error: {}",
                        e
                    ))
                })?;

            // Use citescrape pattern: into_value() without type param, then match
            let text_value = eval_result
                .into_value()
                .map_err(|e| {
                    McpError::Other(anyhow::anyhow!(
                        "Failed to parse result from JavaScript. Error: {}",
                        e
                    ))
                })?;

            // Extract string from serde_json::Value
            let initial_text = if let serde_json::Value::String(text) = text_value {
                text
            } else {
                String::new()
            };

            // If text is empty, likely a SPA where JavaScript hasn't populated innerText
            // Use citescrape's approach: get rendered HTML and convert to text
            if initial_text.trim().is_empty() {
                // Get HTML content (includes JavaScript-rendered DOM)
                let html = page
                    .content()
                    .await
                    .map_err(|e| {
                        McpError::Other(anyhow::anyhow!(
                            "Failed to get HTML content. Error: {}",
                            e
                        ))
                    })?;

                // Convert HTML to markdown/text (removes tags, keeps content)
                // This is citescrape's proven fallback for SPAs
                html2md::parse_html(&html)
            } else {
                initial_text
            }
        };

        let mut contents = Vec::new();

        let source = if args.selector.is_some() { "element" } else { "full page" };

        // ========================================
        // Content[0]: Human-Readable Summary
        // ========================================
        let selector_display = args.selector.as_deref().unwrap_or("full page");
        let preview = if text.chars().count() > 50 {
            format!("{}...", text.chars().take(50).collect::<String>())
        } else {
            text.clone()
        };

        let summary = format!(
            "\x1b[36m󰈙 Extract Text: {}\x1b[0m\n 󰋗 Characters: {} · Preview: {}",
            selector_display,
            text.len(),
            preview
        );
        contents.push(Content::text(summary));

        // ========================================
        // Content[1]: Machine-Parseable JSON
        // ========================================
        let metadata = json!({
            "success": true,
            "text": text,
            "length": text.len(),
            "selector": args.selector,
            "source": source,
            "message": format!(
                "Extracted {} characters from {}",
                text.len(),
                args.selector.as_ref().unwrap_or(&"full page".to_string())
            )
        });
        let json_str = serde_json::to_string_pretty(&metadata)
            .unwrap_or_else(|_| "{}".to_string());
        contents.push(Content::text(json_str));

        Ok(contents)
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![
            PromptArgument {
                name: "use_case".to_string(),
                title: None,
                description: Some(
                    "Type of extraction use case: 'basic', 'selector_patterns', 'spa_content', or 'troubleshooting'"
                        .to_string(),
                ),
                required: Some(false),
            }
        ]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            // ===== Turn 1: Basic Usage =====
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "How do I extract text content from a web page?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use browser_extract_text to get visible text from the current page.\n\n\
                     **Basic Usage (Full Page):**\n\
                     ```json\n\
                     browser_extract_text({})\n\
                     ```\n\
                     Returns all visible text from the page body as a single string.\n\n\
                     **Response Format:**\n\
                     The tool returns two content blocks:\n\
                     1. Human-readable summary: Shows character count and text preview\n\
                     2. JSON metadata with fields:\n\
                        - `success`: true/false\n\
                        - `text`: The extracted text content\n\
                        - `length`: Character count\n\
                        - `selector`: CSS selector used (or null for full page)\n\
                        - `source`: Either \"full page\" or \"element\"\n\
                        - `message`: Status message",
                ),
            },
            
            // ===== Turn 2: CSS Selectors =====
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What if I only want text from a specific element on the page?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use the `selector` parameter with CSS selector syntax:\n\n\
                     **ID-based Selection:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"#article\"})\n\
                     ```\n\n\
                     **Class-based Selection:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \".content\"})\n\
                     browser_extract_text({\"selector\": \".post-body\"})\n\
                     ```\n\n\
                     **Element Type Selection:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"article\"})\n\
                     browser_extract_text({\"selector\": \"main\"})\n\
                     browser_extract_text({\"selector\": \"nav\"})\n\
                     ```\n\n\
                     **Nested/Descendant Selectors:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"article.post p\"})\n\
                     browser_extract_text({\"selector\": \"div.container > p\"})\n\
                     browser_extract_text({\"selector\": \"body > main > section\"})\n\
                     ```\n\n\
                     **Attribute Selectors:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"[data-role=\\\"main-content\\\"]\"})\n\
                     ```",
                ),
            },

            // ===== Turn 3: Advanced Patterns & Specificity =====
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What if my selector is too broad and matches multiple elements, or I need more specific targeting?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use specificity and combinators to narrow down element matches:\n\n\
                     **Combining Class + ID:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"#main.article-content\"})\n\
                     ```\n\n\
                     **Multiple Classes:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \".post.featured.highlighted\"})\n\
                     ```\n\n\
                     **First/Last Child:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"article:first-child\"})\n\
                     browser_extract_text({\"selector\": \"p:last-child\"})\n\
                     ```\n\n\
                     **nth-child:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"li:nth-child(1)\"})\n\
                     browser_extract_text({\"selector\": \"tr:nth-child(odd)\"})\n\
                     ```\n\n\
                     **Combined Ancestor + Descendant:**\n\
                     ```json\n\
                     browser_extract_text({\"selector\": \"div.sidebar ul li\"})\n\
                     ```\n\n\
                     **Important:** The selector returns text from the FIRST matching element. If your selector matches multiple elements, only the first one's text is extracted.",
                ),
            },

            // ===== Turn 4: SPA Content & Async Handling =====
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What happens with Single Page Applications (SPAs) where content loads dynamically?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The tool handles SPAs intelligently with automatic fallback:\n\n\
                     **Standard Flow:**\n\
                     1. Attempts immediate extraction via `document.body.innerText` (works for server-rendered and most SPAs)\n\
                     2. If innerText is empty, fetches rendered HTML via `page.content()`\n\
                     3. Converts HTML to plain text using html2md library\n\n\
                     **Why This Matters:**\n\
                     - **Server-rendered pages (default):** Direct innerText extraction is fast and accurate\n\
                     - **SPAs with pre-rendered content:** innerText includes JS-rendered DOM\n\
                     - **SPAs with async content:** Fallback to HTML conversion captures what's visible\n\n\
                     **Important Considerations:**\n\
                     - Ensure browser has navigated to the page using `browser_navigate` first\n\
                     - If content loads after user interaction, use `browser_click` or `browser_wait` before extracting\n\
                     - Some SPAs require selectors to wait for specific elements: use `browser_navigate` with `wait_for_selector` before extracting\n\n\
                     **Example SPA Workflow:**\n\
                     ```json\n\
                     Step 1: browser_navigate({\"url\": \"https://spa-app.com\", \"wait_for_selector\": \".app-loaded\"})\n\
                     Step 2: browser_wait({\"duration_ms\": 500})\n\
                     Step 3: browser_extract_text({\"selector\": \".content-area\"})\n\
                     ```",
                ),
            },

            // ===== Turn 5: Error Scenarios & Troubleshooting =====
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What happens when my selector doesn't match anything or the page isn't loaded?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The tool provides clear error messages to help troubleshoot:\n\n\
                     **Error: \"Element not found for selector\"**\n\
                     Causes:\n\
                     - Selector CSS syntax is invalid (e.g., missing # or . prefix)\n\
                     - Element doesn't exist on current page\n\
                     - Element is inside an iframe (unsupported - extract from main page instead)\n\
                     - Case sensitivity mismatch in class names\n\n\
                     **Error: \"Failed to get page. Did you call browser_navigate first?\"**\n\
                     Solution: Must navigate to a URL before extracting text\n\
                     ```json\n\
                     browser_navigate({\"url\": \"https://example.com\"})\n\
                     browser_extract_text({})\n\
                     ```\n\n\
                     **Error: \"Failed to get text from selector\"**\n\
                     Possible causes:\n\
                     - Element exists but has no text content\n\
                     - Element is hidden (display: none, visibility: hidden)\n\
                     - Element was removed from DOM after page load\n\
                     - Browser session is in invalid state\n\n\
                     **Best Practices for Error Prevention:**\n\
                     1. First extract full page: `browser_extract_text({})` to verify page is loaded\n\
                     2. Use browser_screenshot to visually confirm elements exist\n\
                     3. Test selectors incrementally (e.g., test \"div.content\" before \"div.content > article > p\")\n\
                     4. Check HTML structure using browser_screenshot with content overlay\n\
                     5. Use simple selectors first (IDs) before complex descendant chains",
                ),
            },

            // ===== Turn 6: Return Format & Integration =====
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "How do I use the extracted text in my workflow? What does the response structure look like?",
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The response contains two content blocks optimized for different use cases:\n\n\
                     **Content[0]: Human-Readable Summary**\n\
                     Example:\n\
                     ```\n\
                     Extract Text: #article\n\
                      Characters: 3245 · Preview: The quick brown fox jumps over the lazy dog. This is...\n\
                     ```\n\n\
                     **Content[1]: Machine-Parseable JSON**\n\
                     ```json\n\
                     {\n\
                       \"success\": true,\n\
                       \"text\": \"The full extracted text content goes here...\",\n\
                       \"length\": 3245,\n\
                       \"selector\": \"#article\",\n\
                       \"source\": \"element\",\n\
                       \"message\": \"Extracted 3245 characters from #article\"\n\
                     }\n\
                     ```\n\n\
                     **Using Extracted Text in Workflows:**\n\
                     1. **Content Analysis:** Extract text, then summarize or analyze with Claude\n\
                        - Use for article summaries, content tagging, sentiment analysis\n\
                     2. **Data Extraction:** Get structured data from tables or lists\n\
                        - Extract table content, then parse with regex or JSON conversion\n\
                     3. **Monitoring:** Compare extracted text across multiple extractions\n\
                        - Detect changes in page content, monitor pricing, track updates\n\
                     4. **Multi-Step Navigation:** Extract after interactions\n\
                        ```json\n\
                        Step 1: browser_navigate({\"url\": \"...\"})\n\
                        Step 2: browser_click({\"selector\": \"a.expandable\"})\n\
                        Step 3: browser_extract_text({\"selector\": \".expanded-content\"})\n\
                        ```\n\n\
                     **Key Integration Points:**\n\
                     - Always navigate first with `browser_navigate`\n\
                     - Use `browser_screenshot` to debug selector issues\n\
                     - Combine with `browser_click` and `browser_type_text` for form interactions\n\
                     - Use `browser_scroll` to access off-screen content before extraction\n\
                     - Parse JSON response to programmatically access text length and selector metadata",
                ),
            },
        ])
    }
}
