// Module declarations
mod config;
mod messaging;
mod processor;
mod browser_state;
mod llm_integration;
mod action_executor;
mod agent;

// Public re-exports (maintains original API)
pub use config::{AgentConfig, PromptConfig};
pub use agent::Agent;
