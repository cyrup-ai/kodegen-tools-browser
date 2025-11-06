use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;
use kodegen_mcp_client::KodegenClient;

use crate::agent::{AgentError, AgentOutput, AgentResult, prompts::{AgentMessagePrompt, SystemPrompt}};
use crate::utils::AgentState;

/// Shared agent state and processing logic (can be Arc-cloned)
pub(super) struct AgentInner {
    pub(super) task: String,
    pub(super) add_infos: String,
    pub(super) mcp_client: Arc<KodegenClient>,
    pub(super) system_prompt: SystemPrompt,
    pub(super) agent_prompt: AgentMessagePrompt,
    pub(super) max_actions_per_step: usize,
    pub(super) agent_state: Arc<Mutex<AgentState>>,
    pub(super) temperature: f64,
    pub(super) max_tokens: u64,
    pub(super) vision_timeout_secs: u64,
    pub(super) llm_timeout_secs: u64,
}

/// Core processing logic
impl AgentInner {
    /// Process a single agent step internally
    pub(super) async fn process_step(&self) -> AgentResult<AgentOutput> {
        // Check if stop requested
        let agent_state = self.agent_state.lock().await;
        if agent_state.is_stop_requested() {
            return Err(AgentError::Stopped);
        }
        drop(agent_state);

        // Get current browser state (with screenshot)
        let mut browser_state = self.get_browser_state().await?;

        // Generate agent actions using CandleFluentAi LLM (with vision analysis if screenshot available)
        let llm_response = self.generate_actions_with_llm(&mut browser_state).await?;

        // Execute actions via MCP hot path
        let (_action_results, errors) = self.execute_actions(llm_response.action.clone()).await?;

        // Log errors if any
        if !errors.is_empty() {
            warn!("Action execution errors: {:?}", errors);
        }

        // Return output with LLM-generated state (no wasteful rebuilding!)
        Ok(AgentOutput {
            current_state: llm_response.current_state,
            action: llm_response.action,
        })
    }
}
