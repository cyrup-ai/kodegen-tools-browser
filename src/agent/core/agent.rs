use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::time::Duration;
use tracing::{debug, error, info, warn};
use kodegen_mcp_client::KodegenClient;

use crate::agent::{AgentError, AgentHistoryList, AgentOutput, AgentResult};
use crate::utils::AgentState;
use super::config::{AgentConfig, PromptConfig};
use super::messaging::{AgentCommand, AgentResponse};
use super::processor::AgentInner;

/// Agent handle for controlling async actor (NOT Clone)
pub struct Agent {
    inner: Arc<AgentInner>,
    command_channel: mpsc::Sender<AgentCommand>,
    response_channel: Mutex<mpsc::Receiver<AgentResponse>>,

    /// Background processor task handle
    ///
    /// Stores the JoinHandle for the spawned agent processor task.
    /// This ensures the task is tracked and can be awaited for graceful shutdown.
    /// Following the pattern from kodegen_tools_citescrape::CrawlSession.
    #[allow(dead_code)]
    processor_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Agent implementation
impl Agent {
    /// Create a new agent instance
    pub fn new(
        task: &str,
        add_infos: &str,
        mcp_client: Arc<KodegenClient>,
        prompts: PromptConfig,
        max_actions_per_step: usize,
        agent_state: Arc<Mutex<AgentState>>,
        config: AgentConfig,
    ) -> AgentResult<Self> {
        // Create channels for command passing
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (resp_tx, resp_rx) = mpsc::channel(32);

        // Create shared inner state (Arc-wrapped)
        let inner = Arc::new(AgentInner {
            task: task.to_string(),
            add_infos: add_infos.to_string(),
            mcp_client,
            system_prompt: prompts.system_prompt,
            agent_prompt: prompts.agent_prompt,
            max_actions_per_step,
            agent_state,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            vision_timeout_secs: config.vision_timeout_secs,
            llm_timeout_secs: config.llm_timeout_secs,
            previous_action_results: Mutex::new(Vec::new()),
        });

        // Spawn processor with Arc-cloned inner and store handle
        let processor_handle = Self::spawn_agent_processor(Arc::clone(&inner), cmd_rx, resp_tx);

        // Return handle with unique receiver ownership
        Ok(Self {
            inner,
            command_channel: cmd_tx,
            response_channel: Mutex::new(resp_rx),
            processor_handle: Some(processor_handle),
        })
    }

    /// Run the agent to perform a task with a maximum number of steps
    pub async fn run(&self, max_steps: usize) -> AgentResult<AgentHistoryList> {
        let mut history = AgentHistoryList::new();

        for step in 0..max_steps {
            debug!("Running agent step {}/{}", step + 1, max_steps);

            // Check if processor was stopped externally
            if !self.is_running() {
                info!("Agent processor stopped externally, exiting run loop");
                break;
            }

            // Check if stop was requested via AgentState
            if self.is_stop_requested().await {
                info!("Agent run stopped as requested");
                break;
            }

            // Run a single step
            match self.run_step().await {
                Ok(output) => {
                    // Record step output
                    let is_done = output
                        .action
                        .iter()
                        .any(|a| a.action.eq_ignore_ascii_case("done"));
                    history.add_step_with_completion(output.clone(), is_done);

                    // Check if agent considers task complete
                    // Protocol: done if any action is "done" or "Done"
                    if is_done {
                        info!("Agent completed task in {} steps", step + 1);
                        break;
                    }
                }
                Err(e) => {
                    error!("Agent step error: {}", e);
                    return Err(e);
                }
            }
        }

        Ok(history)
    }

    /// Run a single agent step
    async fn run_step(&self) -> AgentResult<AgentOutput> {
        // Send command to agent processor
        self.command_channel
            .send(AgentCommand::RunStep)
            .await
            .map_err(|_| AgentError::ChannelClosed("Command channel closed".into()))?;

        // Wait for response (lock mutex to access receiver)
        let mut receiver = self.response_channel.lock().await;
        match receiver.recv().await {
            Some(AgentResponse::StepComplete(output)) => Ok(output),
            Some(AgentResponse::Error(msg)) => Err(AgentError::StepFailed(msg)),
            Some(AgentResponse::Stopped) => Err(AgentError::Stopped),
            None => Err(AgentError::ChannelClosed("Response channel closed".into())),
        }
    }

    /// Check if agent stop was requested
    async fn is_stop_requested(&self) -> bool {
        let agent_state = self.inner.agent_state.lock().await;
        agent_state.is_stop_requested()
    }

    /// Gracefully shut down the agent processor
    ///
    /// Sends Stop command and waits for processor to confirm shutdown.
    /// Returns when processor has fully stopped and cleaned up resources.
    ///
    /// # Errors
    /// - `AgentError::ChannelClosed`: Command channel already closed (processor dead)
    /// - `AgentError::UnexpectedError`: Processor didn't respond within timeout
    /// - `AgentError::UnexpectedError`: Processor sent unexpected response
    pub async fn stop(&self) -> AgentResult<()> {
        debug!("Stopping agent processor");

        // Send stop command
        self.command_channel
            .send(AgentCommand::Stop)
            .await
            .map_err(|_| {
                AgentError::ChannelClosed(
                    "Cannot stop agent: command channel already closed".into(),
                )
            })?;

        // Wait for Stopped confirmation with timeout
        // Pattern adapted from run_step() (lines 171-179)
        let mut receiver = self.response_channel.lock().await;

        match tokio::time::timeout(
            Duration::from_secs(5), // Processor should stop quickly
            receiver.recv(),
        )
        .await
        {
            Ok(Some(AgentResponse::Stopped)) => {
                info!("Agent processor stopped gracefully");
                Ok(())
            }
            Ok(Some(other)) => {
                warn!("Expected Stopped response, got: {:?}", other);
                Err(AgentError::UnexpectedError(
                    "Agent processor sent unexpected response to Stop command".into(),
                ))
            }
            Ok(None) => {
                warn!("Agent response channel closed during stop");
                // Channel closed = processor dead = effectively stopped
                Ok(())
            }
            Err(_) => {
                error!("Agent processor did not respond to Stop within 5 seconds");
                Err(AgentError::UnexpectedError(
                    "Agent processor stop timeout - processor may be stuck".into(),
                ))
            }
        }
    }

    /// Check if agent processor is still running
    ///
    /// Returns `true` if the processor task is active and accepting commands.
    /// Returns `false` if the processor has stopped (command channel closed).
    ///
    /// This is useful for:
    /// - Checking processor state before sending commands
    /// - Polling for processor completion
    /// - Debugging processor lifecycle
    pub fn is_running(&self) -> bool {
        // Processor is running if command channel is still open
        // When processor exits, it drops cmd_rx which closes the channel
        !self.command_channel.is_closed()
    }

    /// Spawn the agent processor task
    fn spawn_agent_processor(
        inner: Arc<AgentInner>,
        mut cmd_rx: mpsc::Receiver<AgentCommand>,
        resp_tx: mpsc::Sender<AgentResponse>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    AgentCommand::RunStep => {
                        let result = inner.process_step().await;

                        // Map result to response
                        let response = match result {
                            Ok(output) => AgentResponse::StepComplete(output),
                            Err(e) => AgentResponse::Error(e.to_string()),
                        };

                        // Send response and only break if channel closed
                        if let Err(e) = resp_tx.send(response).await {
                            error!("Failed to send response: {}", e);
                            break;
                        }
                    }
                    AgentCommand::Stop => {
                        if let Err(e) = resp_tx.send(AgentResponse::Stopped).await {
                            error!("Failed to send stopped response: {}", e);
                        }
                        break;
                    }
                }
            }
            debug!("Agent processor shutting down cleanly");
        })
    }
}
