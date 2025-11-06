use crate::agent::prompts::{AgentMessagePrompt, SystemPrompt};

/// Configuration parameters for agent behavior
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub temperature: f64,
    pub max_tokens: u64,
    pub vision_timeout_secs: u64,
    pub llm_timeout_secs: u64,
}

/// Prompt configuration for agent
#[derive(Debug, Clone)]
pub struct PromptConfig {
    pub system_prompt: SystemPrompt,
    pub agent_prompt: AgentMessagePrompt,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            max_tokens: 4096,
            vision_timeout_secs: 30,
            llm_timeout_secs: 120,
        }
    }
}
