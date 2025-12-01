//! `browser_research` MCP tool implementation with elite terminal design pattern
//!
//! Action-based interface: EXEC/READ/LIST/KILL
//! Session management with connection isolation
//! Timeout with background continuation

use crate::research::ResearchRegistry;
use crate::utils::{DeepResearch, ResearchOptions};
use kodegen_mcp_schema::browser::{
    BrowserResearchAction, BrowserResearchArgs, BrowserResearchPromptArgs, BROWSER_RESEARCH,
};
use kodegen_mcp_tool::{error::McpError, Tool, ToolExecutionContext};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

// =============================================================================
// OUTPUT SCHEMAS
// =============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct BrowserResearchOutput {
    /// Session number
    pub session: u32,
    
    /// Query being researched
    pub query: String,
    
    /// Current results
    pub results: Vec<ResearchResultOutput>,
    
    /// Whether research is complete
    pub completed: bool,
    
    /// Progress summary
    pub summary: String,
    
    /// Time information
    pub elapsed_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResearchResultOutput {
    pub url: String,
    pub title: String,
    pub summary: String,
    pub content_length: usize,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResearchListOutput {
    /// Connection ID
    pub connection_id: String,
    
    /// Active sessions
    pub sessions: Vec<SessionInfo>,
    
    /// Total count
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ResearchKillOutput {
    /// Session number that was killed
    pub session: u32,
    
    /// Success message
    pub message: String,
}

// =============================================================================
// TOOL IMPLEMENTATION
// =============================================================================

#[derive(Clone)]
pub struct BrowserResearchTool {
    browser_manager: Arc<crate::BrowserManager>,
    registry: Arc<OnceCell<ResearchRegistry>>,
}

impl BrowserResearchTool {
    pub fn new(browser_manager: Arc<crate::BrowserManager>) -> Self {
        Self {
            browser_manager,
            registry: Arc::new(OnceCell::new()),
        }
    }
    
    pub async fn get_registry(&self) -> ResearchRegistry {
        self.registry
            .get_or_init(|| async { ResearchRegistry::new() })
            .await
            .clone()
    }
}

impl Tool for BrowserResearchTool {
    type Args = BrowserResearchArgs;
    type PromptArgs = BrowserResearchPromptArgs;

    fn name() -> &'static str {
        BROWSER_RESEARCH
    }

    fn description() -> &'static str {
        "Perform deep web research with session management.\n\n\
         Actions:\n\
         - RESEARCH: Start new research (spawns background task)\n\
         - READ: Check progress of active research\n\
         - LIST: Show all active research sessions\n\
         - KILL: Terminate a running research session (destroys slot)\n\n\
         Example: browser_research({\"action\": \"RESEARCH\", \"query\": \"Rust async patterns\", \"session\": 0})"
    }

    fn read_only() -> bool {
        false
    }

    fn destructive() -> bool {
        false
    }

    fn idempotent() -> bool {
        false
    }

    fn open_world() -> bool {
        true
    }

    async fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> Result<Vec<Content>, McpError> {
        let registry = self.get_registry().await;
        let connection_id = ctx.connection_id().unwrap_or("default");
        
        match args.action {
            BrowserResearchAction::Research => {
                // Validate query
                let query = args.query.ok_or_else(|| {
                    McpError::invalid_arguments("query is required for RESEARCH action")
                })?;
                
                if query.trim().is_empty() {
                    return Err(McpError::invalid_arguments("Research query cannot be empty"));
                }
                
                // Build research options
                let options = Some(ResearchOptions {
                    max_pages: args.max_pages,
                    max_depth: args.max_depth,
                    search_engine: args.search_engine.clone(),
                    include_links: args.include_links,
                    extract_tables: args.extract_tables,
                    extract_images: args.extract_images,
                    timeout_seconds: args.timeout_seconds,
                });
                
                // Create DeepResearch instance
                let research = DeepResearch::new(
                    self.browser_manager.clone(),
                    args.temperature,
                    args.max_tokens,
                );
                
                // Create new session (always fresh for RESEARCH action)
                let session = registry
                    .create(connection_id, args.session, research, query.clone(), options)
                    .await;
                
                // Start research in background
                session.start().await.map_err(McpError::Other)?;
                
                // Fire-and-forget: return immediately
                if args.await_completion_ms == 0 {
                    let output = BrowserResearchOutput {
                        session: args.session,
                        query,
                        results: vec![],
                        completed: false,
                        summary: "Research started in background. Use READ to check progress.".to_string(),
                        elapsed_seconds: None,
                    };
                    
                    let json = serde_json::to_string_pretty(&output)
                        .map_err(|e| McpError::Other(e.into()))?;
                    return Ok(vec![Content::text(json)]);
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
                let session_output = session.read(args.session).await;
                
                // Convert to output format
                let results: Vec<ResearchResultOutput> = session_output
                    .results
                    .iter()
                    .map(|r| ResearchResultOutput {
                        url: r.url.clone(),
                        title: r.title.clone(),
                        summary: r.summary.clone(),
                        content_length: r.content.len(),
                        timestamp: r.timestamp.to_rfc3339(),
                    })
                    .collect();
                
                let output = BrowserResearchOutput {
                    session: args.session,
                    query: session_output.query,
                    results,
                    completed: session_output.completed,
                    summary: if wait_result.is_ok() {
                        session_output.summary
                    } else {
                        format!(
                            "Research timeout after {}ms. {} results so far. Research continues in background.",
                            args.await_completion_ms,
                            session_output.results.len()
                        )
                    },
                    elapsed_seconds: None,
                };
                
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::Other(e.into()))?;
                Ok(vec![Content::text(json)])
            }
            
            BrowserResearchAction::Read => {
                // Get existing session
                let session = registry
                    .get(connection_id, args.session)
                    .await
                    .ok_or_else(|| {
                        McpError::invalid_arguments(format!(
                            "Research session {} not found",
                            args.session
                        ))
                    })?;
                
                // Read current state
                let session_output = session.read(args.session).await;
                
                // Opportunistic cleanup if session completed
                if session_output.completed {
                    let registry_clone = registry.clone();
                    let conn_id = connection_id.to_string();
                    tokio::spawn(async move {
                        let cleaned = registry_clone.cleanup_completed(&conn_id).await;
                        if cleaned > 0 {
                            tracing::info!(
                                "Cleaned up {} completed session(s) for connection {}", 
                                cleaned, 
                                conn_id
                            );
                        }
                    });
                }
                
                // Convert to output format
                let results: Vec<ResearchResultOutput> = session_output
                    .results
                    .iter()
                    .map(|r| ResearchResultOutput {
                        url: r.url.clone(),
                        title: r.title.clone(),
                        summary: r.summary.clone(),
                        content_length: r.content.len(),
                        timestamp: r.timestamp.to_rfc3339(),
                    })
                    .collect();
                
                let output = BrowserResearchOutput {
                    session: args.session,
                    query: session_output.query,
                    results,
                    completed: session_output.completed,
                    summary: session_output.summary,
                    elapsed_seconds: None,
                };
                
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::Other(e.into()))?;
                Ok(vec![Content::text(json)])
            }
            
            BrowserResearchAction::List => {
                // List all sessions for this connection
                let list_output = registry
                    .list(connection_id)
                    .await
                    .map_err(McpError::Other)?;
                
                let output = ResearchListOutput {
                    connection_id: list_output.connection_id,
                    sessions: list_output
                        .sessions
                        .iter()
                        .map(|s| SessionInfo {
                            session: s.session,
                            query: s.query.clone(),
                            completed: s.completed,
                            results_count: s.results_count,
                        })
                        .collect(),
                    total: list_output.total,
                };
                
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::Other(e.into()))?;
                Ok(vec![Content::text(json)])
            }
            
            BrowserResearchAction::Kill => {
                // Get existing session
                let session = registry
                    .get(connection_id, args.session)
                    .await
                    .ok_or_else(|| {
                        McpError::invalid_arguments(format!(
                            "Research session {} not found",
                            args.session
                        ))
                    })?;
                
                // Kill the session
                session.kill().await.map_err(McpError::Other)?;
                
                // Remove from registry
                registry.remove(connection_id, args.session).await;
                
                let output = ResearchKillOutput {
                    session: args.session,
                    message: format!("Research session {} terminated", args.session),
                };
                
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::Other(e.into()))?;
                Ok(vec![Content::text(json)])
            }
        }
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![
            PromptArgument {
                name: "research_depth".to_string(),
                title: None,
                description: Some(
                    "Research depth: 'shallow' (3 pages), 'moderate' (5 pages), 'deep' (15 pages)".to_string(),
                ),
                required: Some(false),
            },
            PromptArgument {
                name: "use_case".to_string(),
                title: None,
                description: Some(
                    "Use case: 'technical', 'news', 'documentation', or 'general'".to_string(),
                ),
                required: Some(false),
            },
        ]
    }

    async fn prompt(&self, args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        let depth = args.research_depth.as_deref().unwrap_or("moderate");
        let use_case = args.use_case.as_deref().unwrap_or("general");
        
        let message = format!(
            "# browser_research - Session-Based Research\n\n\
             ## Actions\n\n\
             ### RESEARCH - Start Research\n\
             ```json\n\
             {{\n  \
               \"action\": \"RESEARCH\",\n  \
               \"session\": 0,\n  \
               \"query\": \"Rust async patterns\",\n  \
               \"max_pages\": 5,\n  \
               \"await_completion_ms\": 300000\n\
             }}\n\
             ```\n\n\
             ### READ - Check Progress\n\
             ```json\n\
             {{\n  \
               \"action\": \"READ\",\n  \
               \"session\": 0\n\
             }}\n\
             ```\n\n\
             ### LIST - Show All Sessions\n\
             ```json\n\
             {{\n  \
               \"action\": \"LIST\"\n\
             }}\n\
             ```\n\n\
             ### KILL - Destroy Research Session\n\
             ```json\n\
             {{\n  \
               \"action\": \"KILL\",\n  \
               \"session\": 0\n\
             }}\n\
             ```\n\n\
             ## Parameters ({depth} research, {use_case} use case)\n\n\
             - `action`: RESEARCH/READ/LIST/KILL (required)\n\
             - `session`: Session slot number (default: 0)\n\
             - `query`: Research topic (required for RESEARCH)\n\
             - `max_pages`: Pages to visit (3-15, default: 5)\n\
             - `await_completion_ms`: Timeout in ms (0=fire-and-forget, default: 300000=5min)\n\
             - `max_depth`: Link depth (1-4, default: 2)\n\
             - `search_engine`: google/bing/duckduckgo (default: google)\n\
             - `temperature`: LLM creativity (0.0-2.0, default: 0.5)\n"
        );
        
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I use browser_research?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(message),
            },
        ])
    }
}
