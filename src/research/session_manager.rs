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
use tokio_util::sync::CancellationToken;

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
    cleanup_token: CancellationToken,
    cleanup_task: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
}

impl ResearchSessionManager {
    /// Get global singleton instance
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<ResearchSessionManager> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            let token = CancellationToken::new();
            let cleanup_handle = Self::spawn_cleanup_task(token.clone());
            Self {
                sessions: DashMap::new(),
                cleanup_token: token,
                cleanup_task: Arc::new(tokio::sync::Mutex::new(Some(cleanup_handle))),
            }
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
    fn spawn_cleanup_task(cancel_token: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        Self::global().cleanup_old_sessions().await;
                    }
                    _ = cancel_token.cancelled() => {
                        log::info!("Cleanup task cancelled");
                        break;
                    }
                }
            }
        })
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

    /// Shutdown cleanup task gracefully
    pub async fn shutdown(&self) -> Result<()> {
        self.cleanup_token.cancel();
        
        // Take the join handle and wait for task with timeout
        let mut task_lock = self.cleanup_task.lock().await;
        if let Some(handle) = task_lock.take() {
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(Ok(())) => {
                    log::info!("Cleanup task stopped successfully");
                }
                Ok(Err(e)) => {
                    log::warn!("Cleanup task panicked: {:?}", e);
                }
                Err(_) => {
                    log::warn!("Cleanup task didn't stop within timeout");
                }
            }
        }
        
        Ok(())
    }
}

// ShutdownHook implementation for MCP server integration
#[cfg(feature = "server")]
use kodegen_server_http::ShutdownHook;

#[cfg(feature = "server")]
impl ShutdownHook for ResearchSessionManager {
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            ResearchSessionManager::shutdown(self).await
        })
    }
}
