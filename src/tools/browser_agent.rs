//! `browser_agent` MCP tool implementation with elite terminal design pattern
//!
//! Action-based interface: EXEC/READ/KILL
//! Session management with connection isolation
//! Timeout with background continuation

use crate::agent::{Agent, AgentConfig, PromptConfig};
use crate::agent::prompts::{AgentMessagePrompt, SystemPrompt};
use crate::agent::registry::AgentRegistry;
use crate::manager::BrowserManager;
use crate::utils::AgentState;
use kodegen_mcp_schema::browser::{
    BrowserAgentAction, BrowserAgentArgs, BrowserAgentOutput, BrowserAgentPromptArgs,
    BrowserAgentStepInfo, BROWSER_AGENT, BROWSER_NAVIGATE,
};
use kodegen_mcp_tool::{error::McpError, Tool, ToolExecutionContext, ToolResponse};
use rmcp::model::{PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OnceCell};

// =============================================================================
// TOOL IMPLEMENTATION
// =============================================================================

#[derive(Clone)]
pub struct BrowserAgentTool {
    _browser_manager: Arc<BrowserManager>,
    server_url: String,
    registry: Arc<OnceCell<AgentRegistry>>,
}

impl BrowserAgentTool {
    pub fn new(browser_manager: Arc<BrowserManager>, server_url: String) -> Self {
        Self {
            _browser_manager: browser_manager,
            server_url,
            registry: Arc::new(OnceCell::new()),
        }
    }
    
    pub async fn get_registry(&self) -> AgentRegistry {
        self.registry
            .get_or_init(|| async { AgentRegistry::new() })
            .await
            .clone()
    }
}

impl Tool for BrowserAgentTool {
    type Args = BrowserAgentArgs;
    type PromptArgs = BrowserAgentPromptArgs;

    fn name() -> &'static str {
        BROWSER_AGENT
    }

    fn description() -> &'static str {
        "Autonomous browser agent with session management.\n\n\
         Actions:\n\
         - PROMPT: Prompt agent with new task (spawns background work)\n\
         - READ: Check progress of active agent\n\
         - KILL: Terminate a running agent (destroys slot)\n\n\
         Example: browser_agent({\"action\": \"PROMPT\", \"task\": \"Find Rust docs\", \"agent\": 0})"
    }

    fn read_only() -> bool {
        false
    }

    fn open_world() -> bool {
        true
    }

    async fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> Result<ToolResponse<BrowserAgentOutput>, McpError> {
        let registry = self.get_registry().await;
        let connection_id = ctx.connection_id().unwrap_or("default");
        
        match args.action {
            BrowserAgentAction::Prompt => {
                // Validate task
                let task = args.task.ok_or_else(|| {
                    McpError::invalid_arguments("task is required for PROMPT action")
                })?;
                
                if task.trim().is_empty() {
                    return Err(McpError::invalid_arguments("Agent task cannot be empty"));
                }
                
                // Create loopback MCP client
                let (mcp_client, _connection) = kodegen_mcp_client::create_streamable_client(
                    &self.server_url,
                    Default::default(),
                )
                .await
                .map_err(|e| {
                    McpError::Other(anyhow::anyhow!(
                        "Failed to create loopback client: {}",
                        e
                    ))
                })?;
                
                // Navigate to start URL if provided
                if let Some(url) = &args.start_url {
                    mcp_client
                        .call_tool(
                            BROWSER_NAVIGATE,
                            json!({
                                "url": url,
                                "timeout_ms": 30000
                            }),
                        )
                        .await
                        .map_err(|e| {
                            McpError::Other(anyhow::anyhow!("Failed to navigate to start URL: {}", e))
                        })?;
                }
                
                // Create agent configuration
                let prompts = PromptConfig {
                    system_prompt: SystemPrompt::new(),
                    agent_prompt: AgentMessagePrompt::new(),
                };
                
                let agent_state = Arc::new(Mutex::new(AgentState::new()));
                
                let config = AgentConfig {
                    temperature: args.temperature,
                    max_tokens: args.max_tokens,
                    vision_timeout_secs: args.vision_timeout_secs,
                    llm_timeout_secs: args.llm_timeout_secs,
                };
                
                let agent = Agent::new(
                    &task,
                    args.additional_info.as_deref().unwrap_or(""),
                    Arc::new(mcp_client),
                    prompts,
                    args.max_actions_per_step as usize,
                    agent_state,
                    config,
                )
                .map_err(|e| McpError::Other(anyhow::anyhow!("Failed to create agent: {}", e)))?;
                
                // Find or create session
                let session = registry
                    .find_or_create(connection_id, args.agent, agent, task.clone(), args.max_steps as usize)
                    .await
                    .map_err(McpError::Other)?;
                
                // Start agent in background (Agent.run is internally async)
                session.start().await.map_err(McpError::Other)?;
                
                // Fire-and-forget: return immediately
                if args.await_completion_ms == 0 {
                    let output = BrowserAgentOutput {
                        agent: args.agent,
                        task: task.clone(),
                        steps_taken: 0,
                        completed: false,
                        error: None,
                        summary: "Agent started in background. Use READ to check progress.".to_string(),
                        history: vec![],
                    };
                    
                    return Ok(ToolResponse::new(
                        "Agent started in background. Use READ to check progress.",
                        output,
                    ));
                }
                
                // Wait with timeout
                let timeout_duration = Duration::from_millis(args.await_completion_ms);
                let wait_result = tokio::time::timeout(timeout_duration, async {
                    // Poll for completion
                    loop {
                        if session.is_complete().await {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                })
                .await;
                
                // Read current state (whether timed out or completed)
                let session_output = session.read(args.agent).await;
                
                // Convert to output format using schema types
                let history: Vec<BrowserAgentStepInfo> = session_output
                    .history
                    .steps
                    .iter()
                    .map(|step| {
                        let actions: Vec<String> = step.output.action
                            .iter()
                            .map(|a| a.action.clone())
                            .collect();
                        BrowserAgentStepInfo {
                            step: step.step,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            actions,
                            summary: step.output.current_state.summary.clone(),
                            complete: step.is_complete,
                        }
                    })
                    .collect();
                
                let display = if wait_result.is_ok() {
                    session_output.summary.clone()
                } else {
                    format!(
                        "Agent timeout after {}ms. {} steps completed. Agent continues in background.",
                        args.await_completion_ms,
                        session_output.history.steps.len()
                    )
                };
                
                let output = BrowserAgentOutput {
                    agent: args.agent,
                    task: session_output.task,
                    steps_taken: session_output.history.steps.len(),
                    completed: session_output.completed,
                    error: session_output.error.clone(),
                    summary: session_output.summary.clone(),
                    history,
                };
                
                Ok(ToolResponse::new(display, output))
            }
            
            BrowserAgentAction::Read => {
                // Get existing session
                let session = registry
                    .get(connection_id, args.agent)
                    .await
                    .ok_or_else(|| {
                        McpError::invalid_arguments(format!(
                            "Agent {} not found",
                            args.agent
                        ))
                    })?;
                
                // Read current state
                let session_output = session.read(args.agent).await;
                
                // Convert to output format using schema types
                let history: Vec<BrowserAgentStepInfo> = session_output
                    .history
                    .steps
                    .iter()
                    .map(|step| {
                        let actions: Vec<String> = step.output.action
                            .iter()
                            .map(|a| a.action.clone())
                            .collect();
                        BrowserAgentStepInfo {
                            step: step.step,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            actions,
                            summary: step.output.current_state.summary.clone(),
                            complete: step.is_complete,
                        }
                    })
                    .collect();
                
                let output = BrowserAgentOutput {
                    agent: args.agent,
                    task: session_output.task.clone(),
                    steps_taken: session_output.history.steps.len(),
                    completed: session_output.completed,
                    error: session_output.error,
                    summary: session_output.summary.clone(),
                    history,
                };
                
                Ok(ToolResponse::new(session_output.summary, output))
            }
            
            BrowserAgentAction::Kill => {
                // Get existing session
                let session = registry
                    .get(connection_id, args.agent)
                    .await
                    .ok_or_else(|| {
                        McpError::invalid_arguments(format!(
                            "Agent {} not found",
                            args.agent
                        ))
                    })?;
                
                // Kill the session
                session.kill().await.map_err(McpError::Other)?;
                
                // Remove from registry
                registry.remove(connection_id, args.agent).await;
                
                let message = format!("Agent {} terminated", args.agent);
                let output = BrowserAgentOutput {
                    agent: args.agent,
                    task: String::new(),
                    steps_taken: 0,
                    completed: true,
                    error: None,
                    summary: message.clone(),
                    history: vec![],
                };
                
                Ok(ToolResponse::new(message, output))
            }
        }
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![PromptArgument {
            name: "focus_area".to_string(),
            title: None,
            description: Some(
                "Focus area: 'research', 'forms', 'workflow', or 'general'".to_string(),
            ),
            required: Some(false),
        }]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        let message = r#"# browser_agent - Session-Based Browser Automation

## Actions

### PROMPT - Prompt Agent with Task
```json
{
  "action": "PROMPT",
  "agent": 0,
  "task": "Find latest Rust version and extract release notes",
  "start_url": "https://www.rust-lang.org/",
  "max_steps": 8,
  "await_completion_ms": 600000
}
```

### READ - Check Progress
```json
{
  "action": "READ",
  "agent": 0
}
```

### KILL - Destroy Agent Session
```json
{
  "action": "KILL",
  "agent": 0
}
```

## Parameters

- `action`: PROMPT/READ/KILL (required)
- `agent`: Agent slot number (default: 0)
- `task`: Task description (required for PROMPT)
- `start_url`: Initial URL (optional)
- `max_steps`: Max iterations (default: 10)
- `await_completion_ms`: Timeout in ms (0=fire-and-forget, default: 600000=10min)
- `temperature`: LLM creativity (0.0-2.0, default: 0.7)
- `max_actions_per_step`: Actions per iteration (default: 3)
- `additional_info`: Extra context (optional)

## Use Cases

**Web Research**: temperature=0.8, max_steps=12
**Form Filling**: temperature=0.5, max_actions_per_step=2
**Multi-page Workflow**: max_steps=15-20
"#;
        
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I use browser_agent?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(message),
            },
        ])
    }
}
