//! Research session management

use crate::utils::{DeepResearch, ResearchOptions, ResearchResult};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Session state for an active research task
#[derive(Clone)]
pub struct ResearchSession {
    /// Underlying research engine
    research: Arc<DeepResearch>,
    
    /// Shared results (updated in background)
    results: Arc<RwLock<Vec<ResearchResult>>>,
    
    /// Total results counter
    total_results: Arc<AtomicUsize>,
    
    /// Background task handle
    task_handle: Arc<RwLock<Option<JoinHandle<Result<()>>>>>,
    
    /// Query being researched
    query: String,
    
    /// Research options
    options: Option<ResearchOptions>,
    
    /// Session completion flag
    completed: Arc<RwLock<bool>>,
}

/// Output from research session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchOutput {
    /// Session number
    pub session: u32,
    
    /// Query being researched
    pub query: String,
    
    /// Current results
    pub results: Vec<ResearchResult>,
    
    /// Whether research is complete
    pub completed: bool,
    
    /// Progress summary
    pub summary: String,
}

impl ResearchSession {
    /// Create a new research session
    pub fn new(research: DeepResearch, query: String, options: Option<ResearchOptions>) -> Self {
        let results = Arc::new(RwLock::new(Vec::new()));
        let total_results = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(RwLock::new(false));
        
        Self {
            research: Arc::new(research),
            results,
            total_results,
            task_handle: Arc::new(RwLock::new(None)),
            query,
            options,
            completed,
        }
    }
    
    /// Start research in background
    pub async fn start(&self) -> Result<()> {
        let research = self.research.clone();
        let query = self.query.clone();
        let options = self.options.clone();
        let results = self.results.clone();
        let total_results = self.total_results.clone();
        let completed = self.completed.clone();
        
        let handle = tokio::spawn(async move {
            match research.research(&query, options, results.clone(), total_results.clone()).await {
                Ok(()) => {
                    let mut comp = completed.write().await;
                    *comp = true;
                    Ok(())
                }
                Err(e) => {
                    let mut comp = completed.write().await;
                    *comp = true;
                    Err(anyhow::anyhow!("Research error: {}", e))
                }
            }
        });
        
        let mut task = self.task_handle.write().await;
        *task = Some(handle);
        
        Ok(())
    }
    
    /// Read current progress
    pub async fn read(&self, session_id: u32) -> ResearchOutput {
        let results = self.results.read().await.clone();
        let completed = *self.completed.read().await;
        
        let summary = if completed {
            format!("Research completed. {} results found.", results.len())
        } else {
            format!("Research in progress. {} results so far.", results.len())
        };
        
        ResearchOutput {
            session: session_id,
            query: self.query.clone(),
            results,
            completed,
            summary,
        }
    }
    
    /// Kill the research task
    pub async fn kill(&self) -> Result<()> {
        let mut task = self.task_handle.write().await;
        if let Some(handle) = task.take() {
            handle.abort();
        }
        
        let mut comp = self.completed.write().await;
        *comp = true;
        
        Ok(())
    }
    
    /// Check if research is complete
    pub async fn is_complete(&self) -> bool {
        *self.completed.read().await
    }
    
    /// Get current results count
    pub async fn results_count(&self) -> usize {
        (*self.results.read().await).len()
    }
}
