//! Browser extract text tool - gets page or element text content

use kodegen_mcp_schema::browser::{BrowserExtractTextArgs, BrowserExtractTextPromptArgs, BROWSER_EXTRACT_TEXT};
use kodegen_mcp_tool::{Tool, error::McpError};
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

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
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
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I get text from a page?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use browser_extract_text to get page content. Examples:\\n\
                     - browser_extract_text({}) - Full page text\\n\
                     - browser_extract_text({\\\"selector\\\": \\\"#article\\\"}) - Specific element by ID\\n\
                     - browser_extract_text({\\\"selector\\\": \\\".content\\\"}) - By class\\n\
                     - browser_extract_text({\\\"selector\\\": \\\"article p\\\"}) - Nested selector\\n\\n\
                     Returns visible text only (no HTML tags).",
                ),
            },
        ])
    }
}
