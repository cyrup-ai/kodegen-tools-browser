use tracing::{debug, info, warn};
use crate::agent::{ActionModel, ActionResult, AgentError, AgentResult};
use super::processor::AgentInner;

/// Action execution implementation
impl AgentInner {
    /// Execute actions by calling existing MCP tools (HOT PATH!)
    ///
    /// Maps agent protocol action names to MCP tool names and parameters.
    /// Each action is translated to an MCP call via self.mcp_client.call_tool().
    ///
    /// Action mapping (agent protocol → MCP tool):
    /// - go_to_url → browser_navigate
    /// - click_element → browser_click  
    /// - input_text → browser_type_text
    /// - scroll → browser_scroll
    /// - extract_page_content → browser_extract_text
    /// - done → (special case, no MCP call)
    ///
    pub(super) async fn execute_actions(
        &self,
        actions: Vec<ActionModel>,
    ) -> AgentResult<(Vec<ActionResult>, Vec<String>)> {
        let mut results = Vec::new();
        let mut errors = Vec::new();

        for action in actions {
            // Map agent action names to MCP tool names (HOT PATH!)
            let (tool_name, tool_args) =
                match action.action.as_str() {
                    "go_to_url" => {
                        let url = action.parameters.get("url").ok_or_else(|| {
                            AgentError::StepFailed("Missing 'url' parameter".into())
                        })?;
                        (
                            "browser_navigate",
                            serde_json::json!({
                                "url": url,
                                "timeout_ms": 30000
                            }),
                        )
                    }
                    "click_element" => {
                        // Support both direct selector and index-based selector
                        // Converts index to [data-mcp-index="N"] selector
                        let selector = if let Some(selector) = action.parameters.get("selector") {
                            selector.clone()
                        } else if let Some(index) = action.parameters.get("index") {
                            // ✅ FIXED: Validate index is numeric before using in selector
                            let index_num = index.parse::<u64>().map_err(|_| {
                                AgentError::StepFailed(format!(
                                    "Invalid index parameter: must be numeric, got '{}'",
                                    index
                                ))
                            })?;
                            format!("[data-mcp-index=\"{}\"]", index_num)
                        } else {
                            return Err(AgentError::StepFailed(
                                "Missing 'selector' or 'index' parameter".into(),
                            ));
                        };
                        (
                            "browser_click",
                            serde_json::json!({
                                "selector": selector,
                                "timeout_ms": 5000
                            }),
                        )
                    }
                    "input_text" => {
                        // Support both direct selector and index-based selector
                        let selector = if let Some(selector) = action.parameters.get("selector") {
                            selector.clone()
                        } else if let Some(index) = action.parameters.get("index") {
                            // ✅ FIXED: Validate index is numeric before using in selector
                            let index_num = index.parse::<u64>().map_err(|_| {
                                AgentError::StepFailed(format!(
                                    "Invalid index parameter: must be numeric, got '{}'",
                                    index
                                ))
                            })?;
                            format!("[data-mcp-index=\"{}\"]", index_num)
                        } else {
                            return Err(AgentError::StepFailed(
                                "Missing 'selector' or 'index' parameter".into(),
                            ));
                        };
                        let text = action.parameters.get("text").ok_or_else(|| {
                            AgentError::StepFailed("Missing 'text' parameter".into())
                        })?;
                        (
                            "browser_type_text",
                            serde_json::json!({
                                "selector": selector,
                                "text": text,
                                "clear": true
                            }),
                        )
                    }
                    "scroll" => {
                        let direction = action
                            .parameters
                            .get("direction")
                            .map(|s| s.as_str())
                            .unwrap_or("down");

                        // Parse scroll amount with default fallback
                        let amount = action
                            .parameters
                            .get("amount")
                            .and_then(|a| a.parse::<i32>().ok())
                            .unwrap_or(500);

                        // Validate and clamp to reasonable range (1-10,000 pixels)
                        // Rationale: Typical viewport is ~1000-2000px tall, 10k = ~5 screen heights
                        let original_amount = amount;
                        let amount = amount.clamp(1, 10_000);

                        // Warn if value was clamped (helps debugging LLM behavior)
                        if original_amount != amount {
                            warn!(
                                "Scroll amount {} out of range [1, 10000], clamped to {}",
                                original_amount, amount
                            );
                        }

                        let (x, y) = match direction {
                            "up" => (0, -amount),
                            "down" => (0, amount),
                            "left" => (-amount, 0),
                            "right" => (amount, 0),
                            _ => (0, amount),
                        };

                        (
                            "browser_scroll",
                            serde_json::json!({
                                "x": x,
                                "y": y
                            }),
                        )
                    }
                    "extract_page_content" => ("browser_extract_text", serde_json::json!({})),
                    "done" => {
                        // Special case: mark completion without MCP call
                        // Agent protocol uses "done" to signal task completion
                        results.push(ActionResult {
                            action: "done".into(),
                            success: true,
                            extracted_content: action
                                .parameters
                                .get("result")
                                .map(|r| r.to_string())
                                .or_else(|| Some("Task completed".into())),
                            error: None,
                        });
                        continue;
                    }
                    _ => {
                        let error_msg = format!("Unknown action: {}", action.action);
                        warn!("Agent attempted unknown action: {}", action.action);
                        errors.push(error_msg.clone());
                        results.push(ActionResult {
                            action: action.action.clone(),
                            success: false,
                            extracted_content: None,
                            error: Some(error_msg),
                        });
                        continue;
                    }
                };

            // Call existing tool via MCP client (HOT PATH!)
            debug!(
                "Agent calling MCP tool: {} with args: {:?}",
                tool_name, tool_args
            );
            match self.mcp_client.call_tool(tool_name, tool_args).await {
                Ok(result) => {
                    info!(
                        "Tool {} succeeded for action '{}': {:?}",
                        tool_name, action.action, result
                    );

                    // Extract meaningful content from tool response
                    // Tools return text content in CallToolResult.content[0].text
                    let content = result
                        .content
                        .first()
                        .and_then(|c| c.as_text())
                        .map(|t| t.text.clone())
                        .unwrap_or_else(|| format!("Tool {} completed", tool_name));

                    results.push(ActionResult {
                        action: action.action,
                        success: true,
                        extracted_content: Some(content),
                        error: None,
                    });
                }
                Err(e) => {
                    let error_msg = format!(
                        "Tool '{}' failed for action '{}': {}",
                        tool_name, action.action, e
                    );
                    warn!("{}", error_msg);
                    errors.push(error_msg.clone());
                    results.push(ActionResult {
                        action: action.action,
                        success: false,
                        extracted_content: None,
                        error: Some(error_msg),
                    });
                }
            }
        }

        Ok((results, errors))
    }
}
