//! Agent session registry with connection isolation

use super::core::Agent;
use super::session::AgentSession;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Registry key: (connection_id, agent_number)
type RegistryKey = (String, u32);

/// Registry for managing multiple agent sessions
pub struct AgentRegistry {
    sessions: Arc<Mutex<HashMap<RegistryKey, Arc<AgentSession>>>>,
}

/// Information about a single agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Agent number
    pub agent: u32,
    
    /// Task being executed
    pub task: String,
    
    /// Whether complete
    pub completed: bool,
    
    /// Whether has error
    pub has_error: bool,
    
    /// Current step count
    pub step_count: usize,
}

impl AgentRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// Find or create an agent session
    pub async fn find_or_create(
        &self,
        connection_id: &str,
        agent_id: u32,
        agent: Agent,
        task: String,
        max_steps: usize,
    ) -> Result<Arc<AgentSession>> {
        let key = (connection_id.to_string(), agent_id);
        let mut sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get(&key) {
            return Ok(session.clone());
        }
        
        // Create new session
        let session = Arc::new(AgentSession::new(agent, task, max_steps));
        sessions.insert(key, session.clone());
        
        Ok(session)
    }
    
    /// Get an existing session
    pub async fn get(
        &self,
        connection_id: &str,
        agent_id: u32,
    ) -> Option<Arc<AgentSession>> {
        let key = (connection_id.to_string(), agent_id);
        let sessions = self.sessions.lock().await;
        sessions.get(&key).cloned()
    }
    
    /// Remove a session (after KILL)
    pub async fn remove(&self, connection_id: &str, agent_id: u32) -> Option<Arc<AgentSession>> {
        let key = (connection_id.to_string(), agent_id);
        let mut sessions = self.sessions.lock().await;
        sessions.remove(&key)
    }
    
    /// List all agent sessions for a connection
    pub async fn list(&self, connection_id: &str) -> Result<Vec<AgentInfo>> {
        let sessions_map = self.sessions.lock().await;
        let mut agent_infos = Vec::new();
        
        for ((conn_id, agent_num), session) in sessions_map.iter() {
            if conn_id == connection_id {
                let completed = session.is_complete().await;
                let has_error = session.has_error().await;
                let step_count = session.step_count().await;
                let output = session.read(*agent_num).await;
                
                agent_infos.push(AgentInfo {
                    agent: *agent_num,
                    task: output.task,
                    completed,
                    has_error,
                    step_count,
                });
            }
        }
        
        // Sort by agent number
        agent_infos.sort_by_key(|a| a.agent);
        
        Ok(agent_infos)
    }
    
    /// Clean up completed sessions (optional maintenance)
    pub async fn cleanup_completed(&self, connection_id: &str) -> usize {
        let mut sessions = self.sessions.lock().await;
        let mut to_remove = Vec::new();
        
        for ((conn_id, agent_num), session) in sessions.iter() {
            if conn_id == connection_id && session.is_complete().await {
                to_remove.push((conn_id.clone(), *agent_num));
            }
        }
        
        let count = to_remove.len();
        for key in to_remove {
            sessions.remove(&key);
        }
        
        count
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
