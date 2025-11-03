//! Research session manager for async browser research operations.
//!
//! This module provides session management for long-running browser research tasks,
//! allowing them to run in the background while clients poll for progress and results.

use anyhow::Result;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

/// Maximum session age before automatic cleanup (5 minutes)
const SESSION_TIMEOUT: Duration = Duration::from_secs(300);

/// Research session status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResearchStatus {
    /// Research is currently running
    Running,
    /// Research completed successfully
    Completed,
    /// Research failed with error
    Failed,
    /// Research was cancelled by user
    Cancelled,
}

/// Progress step during research
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchStep {
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
    /// Progress message
    pub message: String,
    /// Number of pages visited so far
    pub pages_visited: usize,
}

/// Research session state
pub struct ResearchSession {
    /// Unique session identifier
    pub session_id: String,
    /// Research query
    pub query: String,
    /// Current status
    pub status: ResearchStatus,
    /// When session started
    pub started_at: Instant,
    /// Progress steps
    pub progress: Vec<ResearchStep>,
    /// Incremental results as research progresses (matches search pattern)
    pub results: Arc<tokio::sync::RwLock<Vec<crate::utils::ResearchResult>>>,
    /// Completion flag (set when research finishes)
    pub is_complete: Arc<std::sync::atomic::AtomicBool>,
    /// Total results counter for progress tracking
    pub total_results: Arc<std::sync::atomic::AtomicUsize>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Background task handle
    pub task_handle: Option<JoinHandle<()>>,
}

impl ResearchSession {
    /// Create new research session
    pub fn new(session_id: String, query: String) -> Self {
        Self {
            session_id,
            query,
            status: ResearchStatus::Running,
            started_at: Instant::now(),
            progress: Vec::new(),
            results: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            is_complete: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            total_results: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            error: None,
            task_handle: None,
        }
    }

    /// Get runtime in seconds
    pub fn runtime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Add progress step
    pub fn add_progress(&mut self, message: String, pages_visited: usize) {
        self.progress.push(ResearchStep {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            message,
            pages_visited,
        });
    }

    /// Mark as failed
    pub fn fail(&mut self, error: String) {
        self.status = ResearchStatus::Failed;
        self.error = Some(error);
    }

    /// Mark as cancelled
    pub fn cancel(&mut self) {
        self.status = ResearchStatus::Cancelled;
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
    }
}

/// Global research session manager
pub struct ResearchSessionManager {
    sessions: DashMap<String, Arc<tokio::sync::Mutex<ResearchSession>>>,
}

impl ResearchSessionManager {
    /// Get global singleton instance
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<ResearchSessionManager> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            let manager = Self {
                sessions: DashMap::new(),
            };
            // Spawn cleanup task
            manager.spawn_cleanup_task();
            manager
        })
    }

    /// Create new research session
    pub async fn create_session(&self, session_id: String, query: String) -> Result<Arc<tokio::sync::Mutex<ResearchSession>>> {
        let session = Arc::new(tokio::sync::Mutex::new(ResearchSession::new(
            session_id.clone(),
            query,
        )));
        self.sessions.insert(session_id, session.clone());
        Ok(session)
    }

    /// Get session by ID
    pub async fn get_session(&self, session_id: &str) -> Result<Arc<tokio::sync::Mutex<ResearchSession>>> {
        self.sessions
            .get(session_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow::anyhow!("Research session not found: {}", session_id))
    }

    /// Stop session by ID
    pub async fn stop_session(&self, session_id: &str) -> Result<()> {
        let session_ref = self.get_session(session_id).await?;
        let mut session = session_ref.lock().await;
        session.cancel();
        Ok(())
    }

    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
        let mut sessions = Vec::new();
        for entry in self.sessions.iter() {
            if let Ok(session) = entry.value().try_lock() {
                sessions.push(serde_json::json!({
                    "session_id": session.session_id,
                    "query": session.query,
                    "status": session.status,
                    "started_at": session.started_at.elapsed().as_millis() as u64,
                    "runtime_seconds": session.runtime_seconds(),
                    "pages_visited": session.progress.last().map(|p| p.pages_visited).unwrap_or(0),
                    "current_step": session.progress.last().map(|p| p.message.clone()).unwrap_or_default(),
                }));
            }
        }
        sessions
    }

    /// Spawn background cleanup task
    fn spawn_cleanup_task(&self) {
        tokio::spawn(async {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                Self::global().cleanup_old_sessions().await;
            }
        });
    }

    /// Remove sessions older than timeout
    async fn cleanup_old_sessions(&self) {
        let mut to_remove = Vec::new();

        for entry in self.sessions.iter() {
            if let Ok(session) = entry.value().try_lock()
                && session.started_at.elapsed() > SESSION_TIMEOUT
                    && session.status != ResearchStatus::Running {
                    to_remove.push(session.session_id.clone());
                }
        }

        for session_id in to_remove {
            self.sessions.remove(&session_id);
        }
    }
}
