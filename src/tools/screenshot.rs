//! Browser screenshot tool - captures page or element as base64 image

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide_cdp::cdp::browser_protocol::page::CaptureScreenshotFormat;
use kodegen_mcp_schema::browser::{
    BrowserScreenshotArgs, BrowserScreenshotOutput, BrowserScreenshotPromptArgs, BROWSER_SCREENSHOT,
};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, ToolResponse, error::McpError};
use rmcp::model::{PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use std::sync::Arc;

use crate::manager::BrowserManager;

#[derive(Clone)]
pub struct BrowserScreenshotTool {
    manager: Arc<BrowserManager>,
}

impl BrowserScreenshotTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

impl Tool for BrowserScreenshotTool {
    type Args = BrowserScreenshotArgs;
    type PromptArgs = BrowserScreenshotPromptArgs;

    fn name() -> &'static str {
        BROWSER_SCREENSHOT
    }

    fn description() -> &'static str {
        "Take a screenshot of the current page or specific element. Returns base64-encoded image.\\n\\n\
         Example: browser_screenshot({}) for full page\\n\
         Example: browser_screenshot({\\\"selector\\\": \\\"#content\\\"}) for specific element\\n\
         Example: browser_screenshot({\\\"format\\\": \\\"jpeg\\\"}) for smaller file size"
    }

    fn read_only() -> bool {
        true // Screenshots don't modify browser state
    }

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<ToolResponse<BrowserScreenshotOutput>, McpError> {
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

        // Normalize format string for display
        let format_str = match args.format.as_deref() {
            Some("jpeg") | Some("jpg") => "jpeg",
            Some("png") | None => "png",
            _ => "png",
        };

        // Create enum for chromiumoxide API
        let format_enum = match format_str {
            "jpeg" => CaptureScreenshotFormat::Jpeg,
            _ => CaptureScreenshotFormat::Png,
        };

        // Get viewport dimensions before taking screenshot
        let viewport_result = page
            .evaluate("(() => ({ width: window.innerWidth, height: window.innerHeight }))()")
            .await
            .map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Failed to get viewport dimensions: {}",
                    e
                ))
            })?;

        let viewport_width = viewport_result
            .value()
            .and_then(|v| v.get("width"))
            .and_then(|w| w.as_u64())
            .unwrap_or(1920) as u32;

        let viewport_height = viewport_result
            .value()
            .and_then(|v| v.get("height"))
            .and_then(|h| h.as_u64())
            .unwrap_or(1080) as u32;

        // Build screenshot params
        let screenshot_params = ScreenshotParams::builder()
            .format(format_enum.clone())
            .build();

        // Take screenshot (full page or element)
        let image_data = if let Some(selector) = &args.selector {
            // Element screenshot
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

            element.screenshot(format_enum.clone()).await.map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Element screenshot failed for selector '{}'. \
                     Possible causes: (1) Element is not visible or has no dimensions, \
                     (2) Element is obscured or off-screen, \
                     (3) Page is still loading. \
                     Error: {}",
                    selector,
                    e
                ))
            })?
        } else {
            // Full page screenshot
            page.screenshot(screenshot_params).await.map_err(|e| {
                McpError::Other(anyhow::anyhow!(
                    "Page screenshot failed. \
                     Possible causes: (1) Page has not fully loaded, \
                     (2) Page has excessive height or width, \
                     (3) Browser is in an invalid state. \
                     Error: {}",
                    e
                ))
            })?
        };

        // Encode as base64
        let base64_image = BASE64.encode(&image_data);
        let _size_bytes = image_data.len();

        // Terminal summary
        let target = if let Some(ref sel) = args.selector {
            sel.as_str()
        } else {
            "full page"
        };

        let summary = format!(
            "\x1b[36m󰄀 Screenshot: {}\x1b[0m\n 󰈙 Format: {} · Size: {}x{}",
            target,
            format_str.to_uppercase(),
            viewport_width,
            viewport_height
        );

        // Build typed output
        let output = BrowserScreenshotOutput {
            success: true,
            path: None,
            width: viewport_width,
            height: viewport_height,
            format: format_str.to_string(),
            base64: Some(base64_image),
        };

        Ok(ToolResponse::new(summary, output))
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![PromptArgument {
            name: "focus".to_string(),
            title: None,
            description: Some(
                "What aspect to focus on: 'formats' (PNG vs JPEG), 'selectors' (CSS selection), \
                 'elements' (element vs full-page), or 'all' (comprehensive)"
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
                    "I need to take screenshots of web pages. What should I know about using browser_screenshot?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The browser_screenshot tool captures visual content from the current browser page. \
                     After you've navigated to a page with browser_navigate, you can:\n\n\
                     1. FULL PAGE SCREENSHOT (default):\n    \
                     browser_screenshot({})\n\n\
                     2. ELEMENT SCREENSHOT (specific element):\n    \
                     browser_screenshot({\"selector\": \"#content\"})\n    \
                     browser_screenshot({\"selector\": \".form-container\"})\n\n\
                     3. FORMAT SELECTION:\n    \
                     browser_screenshot({\"format\": \"png\"})    # Lossless, larger files, best for text/diagrams\n    \
                     browser_screenshot({\"format\": \"jpeg\"})   # Lossy, smaller files, best for photos\n\n\
                     The tool returns base64-encoded image data suitable for:\n    \
                     - Vision model analysis\n    \
                     - Including in reports or documentation\n    \
                     - Verifying visual appearance"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What are common mistakes when taking screenshots?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Common mistakes to avoid:\n\n\
                     1. NOT NAVIGATING FIRST:\n    \
                     ERROR: browser_screenshot({}) without browser_navigate\n    \
                     FIX: Always call browser_navigate(url) before taking screenshots\n\n\
                     2. INVALID CSS SELECTORS:\n    \
                     ERROR: browser_screenshot({\"selector\": \"my-element\"})  // bare element names don't work\n    \
                     CORRECT SELECTORS:\n      \
                     - browser_screenshot({\"selector\": \"#my-id\"})           // ID selector\n      \
                     - browser_screenshot({\"selector\": \".my-class\"})         // Class selector\n      \
                     - browser_screenshot({\"selector\": \"div.card\"})          // Tag + class\n      \
                     - browser_screenshot({\"selector\": \"button[aria-label='Save']\"})  // Attribute\n\n\
                     3. SELECTING INVISIBLE ELEMENTS:\n    \
                     ERROR: Selector exists but element is display:none or visibility:hidden\n    \
                     ERROR: Selector is inside an iframe (iframes not supported)\n    \
                     ERROR: Element is outside viewport or off-screen\n\n\
                     4. WRONG FORMAT FOR USE CASE:\n    \
                     Use PNG for: UI layouts, code blocks, technical diagrams, charts\n    \
                     Use JPEG for: Photos, images, screenshots with gradients"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "When should I take element screenshots vs full page?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use FULL PAGE screenshots when:\n    \
                     - You need to see overall page layout\n    \
                     - You want to capture everything above the fold\n    \
                     - You're documenting page appearance\n    \
                     - The element is too large or complex\n\n\
                     Use ELEMENT screenshots when:\n    \
                     - You need to isolate a specific component (button, card, form)\n    \
                     - You want to analyze just one element\n    \
                     - You're testing visual consistency\n    \
                     - The full page is too large or contains unwanted context\n\n\
                     Example workflow:\n    \
                     1. browser_navigate({\"url\": \"https://example.com\"})\n    \
                     2. browser_screenshot({})  // See full page\n    \
                     3. browser_screenshot({\"selector\": \".modal\"})  // Isolate modal dialog\n    \
                     4. browser_screenshot({\"selector\": \"form\", \"format\": \"png\"})  // Sharp form screenshot"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text(
                    "What's the typical workflow for taking and analyzing screenshots?"
                ),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "TYPICAL SCREENSHOT WORKFLOW:\n\n\
                     1. NAVIGATE to target page:\n    \
                     browser_navigate({\"url\": \"https://example.com\", \"wait_for_selector\": \".content-loaded\"})\n\n\
                     2. TAKE screenshot to understand structure:\n    \
                     browser_screenshot({\"format\": \"png\"})  // PNG for clarity\n\n\
                     3. INTERACT with page (optional):\n    \
                     browser_click({\"selector\": \".expand-button\"})\n\n\
                     4. CAPTURE UPDATED STATE:\n    \
                     browser_screenshot({\"selector\": \".expanded-content\"})\n\n\
                     5. USE SCREENSHOT for:\n    \
                     - Vision model to analyze what's visible\n    \
                     - Verifying page state after interactions\n    \
                     - Debugging layout or styling issues\n    \
                     - Including in reports or logs\n\n\
                     OPTIMIZATION TIPS:\n    \
                     - Use PNG format (default) for UI - sharper and clearer\n    \
                     - Use JPEG only if file size is critical\n    \
                     - Element screenshots are faster than full page\n    \
                     - Screenshot returns base64 immediately - no streaming delay"
                ),
            },
        ])
    }
}
