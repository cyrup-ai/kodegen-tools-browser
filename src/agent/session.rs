//! Agent session management

use super::core::Agent;
use super::AgentHistoryList;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Session state for an active agent task
#[derive(Clone)]
pub struct AgentSession {
    /// Underlying agent
    agent: Arc<Agent>,
    
    /// Task being executed
    task: String,
    
    /// Maximum steps for execution
    max_steps: usize,
    
    /// Shared history (updated in background)
    history: Arc<RwLock<AgentHistoryList>>,
    
    /// Background task handle
    task_handle: Arc<RwLock<Option<JoinHandle<Result<()>>>>>,
    
    /// Session completion flag
    completed: Arc<RwLock<bool>>,
    
    /// Error state
    error: Arc<RwLock<Option<String>>>,
}

/// Output from agent session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionOutput {
    /// Agent number
    pub agent: u32,
    
    /// Task being executed
    pub task: String,
    
    /// Current history
    pub history: AgentHistoryList,
    
    /// Whether agent is complete
    pub completed: bool,
    
    /// Error message if any
    pub error: Option<String>,
    
    /// Progress summary
    pub summary: String,
}

impl AgentSession {
    /// Create a new agent session
    pub fn new(agent: Agent, task: String, max_steps: usize) -> Self {
        let history = Arc::new(RwLock::new(AgentHistoryList::new()));
        let completed = Arc::new(RwLock::new(false));
        let error = Arc::new(RwLock::new(None));
        
        Self {
            agent: Arc::new(agent),
            task,
            max_steps,
            history,
            task_handle: Arc::new(RwLock::new(None)),
            completed,
            error,
        }
    }
    
    /// Start agent in background
    pub async fn start(&self) -> Result<()> {
        let agent = self.agent.clone();
        let max_steps = self.max_steps;
        let history = self.history.clone();
        let completed = self.completed.clone();
        let error = self.error.clone();
        
        let handle = tokio::spawn(async move {
            match agent.run(max_steps).await {
                Ok(final_history) => {
                    let mut hist = history.write().await;
                    *hist = final_history;
                    let mut comp = completed.write().await;
                    *comp = true;
                    Ok(())
                }
                Err(e) => {
                    let mut err = error.write().await;
                    *err = Some(e.to_string());
                    let mut comp = completed.write().await;
                    *comp = true;
                    Err(anyhow::anyhow!("Agent error: {}", e))
                }
            }
        });
        
        let mut task_handle = self.task_handle.write().await;
        *task_handle = Some(handle);
        
        Ok(())
    }
    
    /// Read current progress
    pub async fn read(&self, agent_id: u32) -> AgentSessionOutput {
        let history = self.history.read().await.clone();
        let completed = *self.completed.read().await;
        let error = self.error.read().await.clone();
        
        let summary = if let Some(ref err) = error {
            format!("Agent failed: {}", err)
        } else if completed {
            format!("Agent completed. {} steps executed.", history.steps.len())
        } else {
            format!("Agent in progress. {} steps so far.", history.steps.len())
        };
        
        AgentSessionOutput {
            agent: agent_id,
            task: self.task.clone(),
            history,
            completed,
            error,
            summary,
        }
    }
    
    /// Kill the agent task
    pub async fn kill(&self) -> Result<()> {
        // First, stop the agent gracefully
        self.agent.stop().await?;
        
        // Then abort the background task
        let mut task = self.task_handle.write().await;
        if let Some(handle) = task.take() {
            handle.abort();
        }
        
        let mut comp = self.completed.write().await;
        *comp = true;
        
        Ok(())
    }
    
    /// Check if agent is complete
    pub async fn is_complete(&self) -> bool {
        *self.completed.read().await
    }
    
    /// Get current step count
    pub async fn step_count(&self) -> usize {
        self.history.read().await.steps.len()
    }
    
    /// Check if agent has error
    pub async fn has_error(&self) -> bool {
        self.error.read().await.is_some()
    }
}
