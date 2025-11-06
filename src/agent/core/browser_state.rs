use base64::Engine;
use tokio::time::Duration;
use tokio_stream::StreamExt;
use tracing::{debug, warn};
use kodegen_candle_agent::prelude::*;
use cyrup_sugars::prelude::MessageChunk;

use crate::agent::{AgentError, AgentResult, BrowserExtractTextResponse, BrowserScreenshotResponse};
use super::processor::AgentInner;

/// Struct to hold browser state, screenshot path, and visual description
#[derive(Debug, Clone)]
pub(super) struct BrowserStateWithScreenshot {
    pub(super) state: String,
    pub(super) screenshot_path: Option<String>,
    pub(super) visual_description: Option<String>,
}

/// Browser state management implementation
impl AgentInner {
    /// Get current browser state for LLM context (HOT PATH!)
    ///
    /// Fetches page content and optional screenshot via MCP tools.
    /// This provides the LLM with current browser context for action planning.
    ///
    /// Uses:
    /// - browser_extract_text: Get page text content
    /// - browser_screenshot: Get base64-encoded screenshot (optional)
    ///
    /// Returns BrowserStateWithScreenshot with text summary and screenshot.
    pub(super) async fn get_browser_state(&self) -> AgentResult<BrowserStateWithScreenshot> {
        // Extract page content via MCP (HOT PATH!)
        let content = match self
            .mcp_client
            .call_tool("browser_extract_text", serde_json::json!({}))
            .await
        {
            Ok(result) => {
                // Parse text from tool response
                // browser_extract_text returns: {"success": true, "text": "...", "length": N, ...}
                result
                    .content
                    .first()
                    .and_then(|c| c.as_text())
                    .and_then(|t| {
                        serde_json::from_str::<BrowserExtractTextResponse>(&t.text)
                            .ok()
                            .map(|response| response.text)
                    })
                    .unwrap_or_else(|| {
                        warn!("Failed to parse browser_extract_text response, using empty content");
                        String::new()
                    })
            }
            Err(e) => {
                warn!("browser_extract_text failed: {}, using empty content", e);
                String::new()
            }
        };

        // Get screenshot via MCP and save to temp file (HOT PATH!)
        let screenshot_path = match self
            .mcp_client
            .call_tool("browser_screenshot", serde_json::json!({}))
            .await
        {
            Ok(result) => {
                // Parse base64 image from tool response
                // ⚠️ CRITICAL: browser_screenshot returns {"image": base64}, NOT {"base64": base64}!
                let screenshot_base64 =
                    result
                        .content
                        .first()
                        .and_then(|c| c.as_text())
                        .and_then(|t| {
                            serde_json::from_str::<BrowserScreenshotResponse>(&t.text)
                                .ok()
                                .map(|response| response.image)
                        });

                // Save base64 to temp file for vision API
                if let Some(base64_data) = screenshot_base64 {
                    // ✅ FIX 1: Move CPU-intensive base64 decode to blocking thread pool
                    let decoded_bytes = tokio::task::spawn_blocking(move || {
                        base64::engine::general_purpose::STANDARD.decode(&base64_data)
                    })
                    .await
                    .map_err(|e| {
                        AgentError::UnexpectedError(format!("Base64 decode task failed: {}", e))
                    })?
                    .map_err(|e| {
                        AgentError::UnexpectedError(format!("Base64 decode failed: {}", e))
                    })?;

                    // Create unique temp file path with nanosecond precision + PID
                    let temp_dir = std::env::temp_dir();
                    let duration = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(|e| {
                            AgentError::BrowserError(format!("System time error: {}", e))
                        })?;

                    let filename = format!(
                        "browser_screenshot_{}_{:09}_{}.png",
                        duration.as_secs(),
                        duration.subsec_nanos(),
                        std::process::id()
                    );
                    let temp_path = temp_dir.join(filename);

                    // ✅ FIX 2: Use async file write instead of blocking std::fs::write
                    match tokio::fs::write(&temp_path, decoded_bytes).await {
                        Ok(_) => Some(temp_path.to_string_lossy().to_string()),
                        Err(e) => {
                            warn!("Failed to write screenshot to file: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            }
            Err(e) => {
                warn!(
                    "browser_screenshot failed: {}, continuing without screenshot",
                    e
                );
                None
            }
        };

        // Build state representation for LLM
        let state = format!(
            "Content Length: {} characters\nContent Sample: {}{}",
            content.len(),
            &content[0..content.len().min(500)],
            if content.len() > 500 { "..." } else { "" }
        );

        // Store state for recovery if needed
        let mut agent_state = self.agent_state.lock().await;
        agent_state.set_last_valid_state(state.clone());
        drop(agent_state);

        Ok(BrowserStateWithScreenshot {
            state,
            screenshot_path,
            visual_description: None,
        })
    }

    /// Format browser state with vision-based screenshot analysis
    ///
    /// Uses CandleFluentAi::vision() to analyze screenshots and generate
    /// detailed visual descriptions of UI elements and layout.
    ///
    /// Populates browser_state.visual_description with the vision analysis result
    /// for potential caching/reuse.
    pub(super) async fn format_browser_state_with_vision(
        &self,
        browser_state: &mut BrowserStateWithScreenshot,
    ) -> AgentResult<String> {
        let mut state_description = format!("Current browser state:\n{}", browser_state.state);

        // Add vision-based screenshot analysis if available
        if let Some(screenshot_path) = &browser_state.screenshot_path {
            state_description.push_str("\n\nVisual Analysis:\n");

            // Check if we already have cached visual description
            let visual_desc = if let Some(ref cached) = browser_state.visual_description {
                debug!("Using cached visual description");
                cached.clone()
            } else {
                // Generate new vision analysis
                let vision_query = "Describe the visible UI elements, their layout, and any interactive components (buttons, links, forms, input fields, etc.) in detail.";

                // Wrap entire stream consumption in timeout
                let vision_timeout = Duration::from_secs(self.vision_timeout_secs);
                let result = tokio::time::timeout(vision_timeout, async {
                    let mut description = String::with_capacity(4096);
                    let mut stream =
                        CandleFluentAi::vision().describe_image(screenshot_path, vision_query);

                    while let Some(chunk) = stream.next().await {
                        if let Some(error) = chunk.error() {
                            return Err(format!("Vision analysis error: {}", error));
                        }

                        if !chunk.text.is_empty() {
                            description.push_str(&chunk.text);
                        }

                        if chunk.is_final {
                            if let Some(stats) = &chunk.stats {
                                debug!(
                                    "Vision analysis: {} tokens generated",
                                    stats.tokens_generated
                                );
                            }
                            return Ok(description);
                        }
                    }
                    Err("Vision stream ended without final chunk".to_string())
                })
                .await;

                match result {
                    Ok(Ok(desc)) => {
                        browser_state.visual_description = Some(desc.clone());
                        desc
                    }
                    Ok(Err(e)) => {
                        warn!("Vision analysis failed: {}", e);
                        format!("[Vision analysis failed: {}]", e)
                    }
                    Err(_) => {
                        warn!(
                            "Vision analysis timed out after {}s",
                            self.vision_timeout_secs
                        );
                        format!(
                            "[Vision analysis timed out after {}s]",
                            self.vision_timeout_secs
                        )
                    }
                }
            };

            state_description.push_str(&visual_desc);
            state_description.push('\n');

            // Clean up temp screenshot file after vision analysis completes
            if let Err(e) = tokio::fs::remove_file(screenshot_path).await {
                warn!(
                    "Failed to cleanup screenshot file {}: {}",
                    screenshot_path, e
                );
            }
        }

        Ok(state_description)
    }
}
