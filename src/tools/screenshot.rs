//! Browser screenshot tool - captures page or element as base64 image

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide_cdp::cdp::browser_protocol::page::CaptureScreenshotFormat;
use kodegen_mcp_schema::browser::{
    BrowserScreenshotArgs, BrowserScreenshotOutput, BROWSER_SCREENSHOT,
    ScreenshotPrompts,
};
use kodegen_mcp_schema::{Tool, ToolExecutionContext, ToolResponse, McpError};
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
    type Prompts = ScreenshotPrompts;

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
}
