use tokio::time::Duration;
use tokio_stream::StreamExt;
use tracing::{debug, warn};
use kodegen_candle_agent::prelude::*;

use crate::agent::{AgentError, AgentLLMResponse, AgentResult};
use super::processor::AgentInner;
use super::browser_state::BrowserStateWithScreenshot;

/// LLM integration implementation
impl AgentInner {
    /// Generate actions using CandleFluentAi LLM
    ///
    /// Combines system prompt, task description, and browser state into a query,
    /// then streams the LLM response and parses actions from it.
    pub(super) async fn generate_actions_with_llm(
        &self,
        browser_state: &mut BrowserStateWithScreenshot,
    ) -> AgentResult<AgentLLMResponse> {
        // Build browser state message with vision analysis
        let browser_state_msg = self.format_browser_state_with_vision(browser_state).await?;

        // Build system prompt with available actions
        let actions_description = r##"Available Actions:
- go_to_url: Navigate to a URL (parameters: url)
- click_element: Click an element (parameters: selector OR index)
- input_text: Type text into an element (parameters: selector OR index, text)
- scroll: Scroll the page (parameters: direction ["up"|"down"|"left"|"right"], amount [pixels])
- extract_page_content: Extract page text content (no parameters)
- done: Mark task as complete (parameters: result [description of completion])

Parameter Notes:
- selector: CSS selector string (e.g., "#submit", ".button", "input[name='email']")
- index: Numeric index for data-mcp-index attributes (converted to selector automatically)
- Use selector for precision, index for LLM-generated element references

You must respond with valid JSON matching the AgentLLMResponse schema with an 'action' array."##;

        let system_prompt = format!(
            "{}\n\n{}\n\nYou are a browser automation agent. Analyze the browser state and generate appropriate actions.",
            self.system_prompt.build_prompt(),
            actions_description
        );

        // Build user query using AgentMessagePrompt (CRITICAL: integrates agent_prompt field)
        // This provides protocol-compliant instructions and proper context formatting
        let user_query =
            self.agent_prompt
                .build_message_prompt(&browser_state_msg, &self.task, &self.add_infos);

        // Stream LLM response with timeout protection
        let llm_timeout = Duration::from_secs(self.llm_timeout_secs);
        let full_response = match tokio::time::timeout(llm_timeout, async {
            // Pre-allocate based on max_tokens parameter
            // Average: ~4 bytes per token for English text
            let expected_bytes = (self.max_tokens as usize) * 4;
            let mut response = String::with_capacity(expected_bytes);
            let mut stream = CandleFluentAi::agent_role("browser-agent")
                .temperature(self.temperature)
                .max_tokens(self.max_tokens)
                .system_prompt(&system_prompt)
                .into_agent()
                .map_err(|e| AgentError::UnexpectedError(e.to_string()))?
                .chat(move |_conversation| {
                    let query = user_query.clone();
                    async move { CandleChatLoop::UserPrompt(query) }
                })
                .map_err(|e| AgentError::LlmError(e.to_string()))?;

            // Collect streaming response
            while let Some(chunk) = stream.next().await {
                match chunk {
                    CandleMessageChunk::Text(text) => {
                        response.push_str(&text);
                    }
                    CandleMessageChunk::Complete {
                        token_count,
                        elapsed_secs,
                        ..
                    } => {
                        if let (Some(tokens), Some(elapsed)) = (token_count, elapsed_secs) {
                            debug!("LLM generated {} tokens in {:.2}s", tokens, elapsed);
                        }
                        return Ok(response);
                    }
                    CandleMessageChunk::Error(err) => {
                        return Err(AgentError::LlmError(err.to_string()));
                    }
                    _ => {}
                }
            }
            // Stream ended without Complete chunk
            Err(AgentError::LlmError(
                "LLM stream ended without Complete chunk".into(),
            ))
        })
        .await
        {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(AgentError::LlmError(format!(
                    "LLM generation timed out after {}s",
                    self.llm_timeout_secs
                )));
            }
        };

        // Parse actions from JSON response
        let agent_response: AgentLLMResponse =
            serde_json::from_str(&full_response).map_err(|e| {
                AgentError::LlmError(format!(
                    "Failed to parse LLM response as JSON: {}. Response: {}",
                    e, full_response
                ))
            })?;

        // Limit the number of actions
        let limited_actions = if agent_response.action.len() > self.max_actions_per_step {
            warn!(
                "Agent generated {} actions, limiting to {}",
                agent_response.action.len(),
                self.max_actions_per_step
            );
            agent_response.action[0..self.max_actions_per_step].to_vec()
        } else {
            agent_response.action
        };

        Ok(AgentLLMResponse {
            current_state: agent_response.current_state,
            action: limited_actions,
        })
    }
}
