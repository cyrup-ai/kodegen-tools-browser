//! Browser type text tool - inputs text into form fields

use kodegen_mcp_schema::browser::{
    BrowserTypeTextArgs, BrowserTypeOutput, BrowserTypeTextPromptArgs, BROWSER_TYPE_TEXT,
};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, ToolResponse, error::McpError};
use rmcp::model::{PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
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
    type PromptArgs = BrowserTypeTextPromptArgs;

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

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![
            PromptArgument {
                name: "focus_area".to_string(),
                title: None,
                description: Some(
                    "Optional focus area for examples (e.g., 'selectors', 'clearing', 'sensitive_fields', 'complex_forms')"
                        .to_string(),
                ),
                required: Some(false),
            }
        ]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            // Introduction: Basic usage
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "How do I type text into form fields on a web page?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use browser_type_text to input text into form fields using CSS selectors.\n\n\
                     Basic syntax:\n\
                     browser_type_text({\"selector\": \"<css-selector>\", \"text\": \"<text-to-type>\"})\n\n\
                     Examples by field type:\n\
                     1. By ID: browser_type_text({\"selector\": \"#email\", \"text\": \"user@example.com\"})\n\
                     2. By name: browser_type_text({\"selector\": \"input[name='username']\", \"text\": \"john\"})\n\
                     3. By class: browser_type_text({\"selector\": \".search-field\", \"text\": \"query terms\"})\n\
                     4. Complex: browser_type_text({\"selector\": \"form .input-group > input[type='text']\", \"text\": \"data\"})\n\n\
                     The tool automatically focuses the element and clears existing text by default."
                ),
            },
            // Key feature: Clear vs Append
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What's the difference between clearing and appending text? When should I use clear: false?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The 'clear' parameter controls whether existing text is replaced or appended:\n\n\
                     Default behavior (clear: true) - replaces all text:\n\
                     browser_type_text({\"selector\": \"#field\", \"text\": \"new value\"})\n\
                     Result: Field contains only 'new value'\n\n\
                     Append mode (clear: false) - adds text without clearing:\n\
                     browser_type_text({\"selector\": \"#field\", \"text\": \", more text\", \"clear\": false})\n\
                     Result: Field appends ', more text' to existing content\n\n\
                     Use cases:\n\
                     - clear: true - Login forms, single-value inputs, search boxes\n\
                     - clear: false - Editing existing text, appending to a list in a textarea, building complex input\n\n\
                     Common pattern for textareas:\n\
                     1. First call: browser_type_text({\"selector\": \"#notes\", \"text\": \"First line\"})\n\
                     2. Second call: browser_type_text({\"selector\": \"#notes\", \"text\": \"\\nSecond line\", \"clear\": false})"
                ),
            },
            // Advanced: Selector targeting strategies
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What CSS selectors work best for different types of form fields?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "CSS selectors are the foundation of targeting form fields. Here are effective patterns:\n\n\
                     ID selectors (most reliable):\n\
                     - #email, #password, #search\n\
                     - Use when IDs are stable and unique\n\n\
                     Attribute selectors (flexible, XPath-like):\n\
                     - input[type='email']\n\
                     - input[name='username']\n\
                     - textarea[data-field='notes']\n\
                     - button[aria-label='Search']\n\n\
                     Class selectors (common in modern frameworks):\n\
                     - .search-input, .form-control, .login-field\n\
                     - Be careful: same classes may appear multiple times\n\n\
                     Combinators (for uniqueness in complex forms):\n\
                     - form.login input[type='email'] (nested)\n\
                     - .modal input#email (parent + specific field)\n\
                     - section:nth-child(2) input (positional)\n\n\
                     Pseudo-selectors (for specific element states):\n\
                     - input:not([disabled])\n\
                     - input:focus\n\
                     - input:required\n\n\
                     Best practice: Always verify selector specificity:\n\
                     Use browser_extract_text({\"selector\": \"<selector>\", \"attribute\": \"id\"}) or browser_screenshot to check."
                ),
            },
            // Handling challenges: Timeouts and timing
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What should I do if typing fails? How do I handle slow-loading pages?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The tool includes built-in timeout handling to wait for elements on slow/SPA pages:\n\n\
                     Default behavior (5 second timeout):\n\
                     browser_type_text({\"selector\": \"#slow-loading-field\", \"text\": \"input\"})\n\
                     - Waits up to 5000ms for element to appear\n\
                     - Automatically scrolls element into view\n\
                     - Clicks to focus the element first\n\n\
                     Custom timeout for very slow pages:\n\
                     browser_type_text({\n\
                       \"selector\": \"#deep-ajax-field\",\n\
                       \"text\": \"data\",\n\
                       \"timeout_ms\": 15000  // Wait up to 15 seconds\n\
                     })\n\n\
                     Common failure scenarios and solutions:\n\n\
                     1. Element not found (timeout expired):\n\
                        - Verify selector is correct: browser_screenshot() then inspect\n\
                        - Increase timeout_ms if page loads slowly\n\
                        - Ensure browser_navigate() was called first\n\n\
                     2. Element exists but not focusable:\n\
                        - Element may be disabled: browser_screenshot() to check disabled attribute\n\
                        - Element may be hidden: Use visibility checker or adjust selector\n\
                        - Try clicking parent container first\n\n\
                     3. Text appears but wrong value:\n\
                        - Page may have client-side validation that interferes\n\
                        - Try clear: false and clear manually with JS first\n\
                        - Some fields may have input masks or formatters\n\n\
                     4. Element loses focus during typing:\n\
                        - Often happens with auto-complete or validation fields\n\
                        - Check if field has JavaScript event listeners\n\
                        - May need to disable validation or use alternative selector\n\n\
                     Debugging pattern:\n\
                     1. Take screenshot: browser_screenshot({\"filename\": \"before_type.png\"})\n\
                     2. Try typing: browser_type_text({\"selector\": \"...\", \"text\": \"...\"})\n\
                     3. Take screenshot: browser_screenshot({\"filename\": \"after_type.png\"})\n\
                     4. Extract state: browser_extract_text({\"selector\": \"input\", \"attribute\": \"value\"})"
                ),
            },
            // Workflow integration: Form filling patterns
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "Can you show me a complete form-filling workflow example?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Here's a realistic multi-step form filling workflow:\n\n\
                     Typical login form:\n\
                     1. browser_navigate({\"url\": \"https://example.com/login\"})\n\
                     2. browser_type_text({\"selector\": \"#username\", \"text\": \"john@example.com\"})\n\
                     3. browser_type_text({\"selector\": \"#password\", \"text\": \"secretpassword\"})\n\
                     4. browser_click({\"selector\": \"#login-button\"})\n\
                     5. browser_navigate({\"url\": \"expected_next_page\", \"wait_for_selector\": \".dashboard\"})\n\n\
                     Multi-field registration form:\n\
                     1. browser_navigate({\"url\": \"https://example.com/register\"})\n\
                     2. browser_type_text({\"selector\": \"input[name='firstName']\", \"text\": \"John\"})\n\
                     3. browser_type_text({\"selector\": \"input[name='lastName']\", \"text\": \"Doe\"})\n\
                     4. browser_type_text({\"selector\": \"input[type='email']\", \"text\": \"john@example.com\"})\n\
                     5. browser_type_text({\"selector\": \"input[type='password']\", \"text\": \"MySecure123!\"})\n\
                     6. browser_click({\"selector\": \"input[type='checkbox']\"})\n\
                     7. browser_click({\"selector\": \"button[type='submit']\"})\n\n\
                     Search with dynamic results:\n\
                     1. browser_navigate({\"url\": \"https://example.com/search\"})\n\
                     2. browser_type_text({\"selector\": \".search-box\", \"text\": \"keyword\"})\n\
                     3. browser_navigate({\"url\": \"results_page\", \"wait_for_selector\": \".result-item\"})\n\
                     4. browser_extract_text({\"selector\": \".result-item\"})\n\n\
                     Key workflow principles:\n\
                     - Always verify with screenshots between critical operations\n\
                     - Use wait_for_selector in browser_navigate to ensure pages load\n\
                     - Chain related operations but don't assume instant updates\n\
                     - Clear field contents if repurposing the same field\n\
                     - Handle JavaScript validation errors gracefully"
                ),
            },
            // Edge cases and gotchas
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What edge cases should I watch out for when typing text?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Browser automation encounters several edge cases with form fields:\n\n\
                     1. Input masking fields (phone, credit card, date):\n\
                        Input: browser_type_text({\"selector\": \"#phone\", \"text\": \"5551234567\"})\n\
                        Result: May appear as (555) 123-4567 or 555-123-4567 depending on mask\n\
                        Solution: Type with expected formatting or verify extracted value\n\n\
                     2. Auto-complete/suggestion overlays:\n\
                        Issue: Field gets focus, suggestions appear, typing may select wrong item\n\
                        Solution: Let auto-complete settle, or use arrow keys + Enter after typing\n\
                        Pattern: browser_type_text({\"selector\": \"#search\", \"text\": \"term\"})\n\
                                 browser_key_press({\"key\": \"Escape\"}) // Dismiss autocomplete\n\n\
                     3. Required fields with client validation:\n\
                        Issue: Some validators prevent typing until other fields are filled\n\
                        Solution: Fill dependent fields first in correct order\n\n\
                     4. Read-only or disabled fields:\n\
                        Issue: browser_type_text will fail with clear error message\n\
                        Solution: Check field state before attempting to type\n\
                        Check: browser_extract_text({\"selector\": \"#field\", \"attribute\": \"disabled\"})\n\n\
                     5. Contenteditable divs (rich text, not <input>):\n\
                        Issue: CSS selector finds the div but it may not work like input field\n\
                        Solution: browser_type_text works with contenteditable, but behavior differs\n\n\
                     6. Very long text (thousands of characters):\n\
                        Issue: May timeout or hit browser limits\n\
                        Solution: Break into chunks with clear: false\n\n\
                     7. Special characters and encoding:\n\
                        Issue: Unicode, quotes, backslashes may need escaping in JSON\n\
                        Solution: JSON-encode properly before sending to tool\n\
                        Example: browser_type_text({\"selector\": \"#field\", \"text\": \"John\\\"The Great\\\"Doe\"})\n\n\
                     8. Fields that clear themselves:\n\
                        Issue: OTP fields, countdown timers, auto-clearing search\n\
                        Solution: Use clear: false and append immediately, or re-verify\n\n\
                     Pro tip: Always use browser_extract_text after typing to verify:\n\
                     browser_extract_text({\"selector\": \"#field\", \"attribute\": \"value\"})"
                ),
            },
        ])
    }
}
