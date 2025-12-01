//! Research session registry with connection isolation

use super::session::ResearchSession;
use crate::utils::{DeepResearch, ResearchOptions};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Registry key: (connection_id, session_number)
type RegistryKey = (String, u32);

/// Registry for managing multiple research sessions
#[derive(Clone)]
pub struct ResearchRegistry {
    sessions: Arc<Mutex<HashMap<RegistryKey, Arc<ResearchSession>>>>,
}

/// List output showing all active research sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchListOutput {
    /// Connection ID
    pub connection_id: String,
    
    /// Active sessions
    pub sessions: Vec<SessionInfo>,
    
    /// Total count
    pub total: usize,
}

/// Information about a single session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Session number
    pub session: u32,
    
    /// Query being researched
    pub query: String,
    
    /// Whether complete
    pub completed: bool,
    
    /// Current results count
    pub results_count: usize,
}

impl ResearchRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// Create a new research session, replacing any existing one with same ID
    ///
    /// RESEARCH action always starts fresh research. If a session already exists
    /// with the same ID, it is killed and replaced.
    pub async fn create(
        &self,
        connection_id: &str,
        session_id: u32,
        research: DeepResearch,
        query: String,
        options: Option<ResearchOptions>,
    ) -> Arc<ResearchSession> {
        let key = (connection_id.to_string(), session_id);
        let mut sessions = self.sessions.lock().await;

        // Kill existing session if present
        if let Some(old) = sessions.remove(&key) {
            let _ = old.kill().await;
        }

        // Create new session
        let session = Arc::new(ResearchSession::new(research, query, options));
        sessions.insert(key, session.clone());

        session
    }
    
    /// Get an existing session
    pub async fn get(
        &self,
        connection_id: &str,
        session_id: u32,
    ) -> Option<Arc<ResearchSession>> {
        let key = (connection_id.to_string(), session_id);
        let sessions = self.sessions.lock().await;
        sessions.get(&key).cloned()
    }
    
    /// Remove a session (after KILL)
    pub async fn remove(&self, connection_id: &str, session_id: u32) -> Option<Arc<ResearchSession>> {
        let key = (connection_id.to_string(), session_id);
        let mut sessions = self.sessions.lock().await;
        sessions.remove(&key)
    }
    
    /// List all sessions for a connection
    pub async fn list(&self, connection_id: &str) -> Result<ResearchListOutput> {
        let sessions_map = self.sessions.lock().await;
        let mut session_infos = Vec::new();
        
        for ((conn_id, session_num), session) in sessions_map.iter() {
            if conn_id == connection_id {
                let completed = session.is_complete().await;
                let results_count = session.results_count().await;
                let output = session.read(*session_num).await;
                
                session_infos.push(SessionInfo {
                    session: *session_num,
                    query: output.query,
                    completed,
                    results_count,
                });
            }
        }
        
        // Sort by session number
        session_infos.sort_by_key(|s| s.session);
        
        let total = session_infos.len();
        
        Ok(ResearchListOutput {
            connection_id: connection_id.to_string(),
            sessions: session_infos,
            total,
        })
    }
    
    /// Clean up completed sessions (optional maintenance)
    pub async fn cleanup_completed(&self, connection_id: &str) -> usize {
        let mut sessions = self.sessions.lock().await;
        let mut to_remove = Vec::new();
        
        for ((conn_id, session_num), session) in sessions.iter() {
            if conn_id == connection_id && session.is_complete().await {
                to_remove.push((conn_id.clone(), *session_num));
            }
        }
        
        let count = to_remove.len();
        for key in to_remove {
            sessions.remove(&key);
        }
        
        count
    }

    /// Cleanup all research sessions for a connection
    ///
    /// Called when a connection drops to cleanup all associated research sessions.
    /// Returns the number of sessions cleaned up.
    pub async fn cleanup_connection(&self, connection_id: &str) -> usize {
        let mut sessions = self.sessions.lock().await;

        // Collect session IDs to remove
        let to_remove: Vec<RegistryKey> = sessions
            .keys()
            .filter(|(conn_id, _)| conn_id == connection_id)
            .cloned()
            .collect();

        let count = to_remove.len();

        // Remove and drop each session
        for key in to_remove {
            if let Some(session) = sessions.remove(&key) {
                log::debug!(
                    "Cleaning up research session {} for connection {}",
                    key.1,
                    connection_id
                );
                drop(session);
            }
        }

        count
    }
}

impl Default for ResearchRegistry {
    fn default() -> Self {
        Self::new()
    }
}
