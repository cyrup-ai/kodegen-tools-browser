//! `browser_research` MCP tool implementation with elite terminal design pattern
//!
//! Action-based interface: EXEC/READ/LIST/KILL
//! Session management with connection isolation
//! Timeout with background continuation

use crate::research::ResearchRegistry;
use crate::utils::{DeepResearch, ResearchOptions};
use kodegen_mcp_schema::browser::{
    BrowserResearchAction, BrowserResearchArgs, BrowserResearchOutput,
    ResearchSource, BROWSER_RESEARCH,
    ResearchPrompts,
};
use kodegen_mcp_schema::{McpError, Tool, ToolExecutionContext, ToolResponse};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

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
    type Prompts = ResearchPrompts;

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
    ) -> Result<ToolResponse<BrowserResearchOutput>, McpError> {
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
                        status: "running".to_string(),
                        query,
                        pages_analyzed: 0,
                        max_pages: args.max_pages,
                        completed: false,
                        summary: None,
                        key_findings: None,
                        sources: vec![],
                        error: None,
                    };
                    
                    return Ok(ToolResponse::new(
                        "Research started in background. Use READ to check progress.",
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
                let session_output = session.read(args.session).await;
                
                // Convert to output format using schema types
                let sources: Vec<ResearchSource> = session_output
                    .results
                    .iter()
                    .map(|r| ResearchSource {
                        url: r.url.clone(),
                        title: Some(r.title.clone()),
                        summary: Some(r.summary.clone()),
                    })
                    .collect();
                
                let display = if wait_result.is_ok() {
                    session_output.summary.clone()
                } else {
                    format!(
                        "Research timeout after {}ms. {} results so far. Research continues in background.",
                        args.await_completion_ms,
                        session_output.results.len()
                    )
                };
                
                let output = BrowserResearchOutput {
                    session: args.session,
                    status: if session_output.completed { "completed" } else { "running" }.to_string(),
                    query: session_output.query,
                    pages_analyzed: session_output.results.len(),
                    max_pages: args.max_pages,
                    completed: session_output.completed,
                    summary: if session_output.completed { Some(session_output.summary.clone()) } else { None },
                    key_findings: None,
                    sources,
                    error: None,
                };
                
                Ok(ToolResponse::new(display, output))
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
                
                // Convert to output format using schema types
                let sources: Vec<ResearchSource> = session_output
                    .results
                    .iter()
                    .map(|r| ResearchSource {
                        url: r.url.clone(),
                        title: Some(r.title.clone()),
                        summary: Some(r.summary.clone()),
                    })
                    .collect();
                
                let output = BrowserResearchOutput {
                    session: args.session,
                    status: if session_output.completed { "completed" } else { "running" }.to_string(),
                    query: session_output.query.clone(),
                    pages_analyzed: session_output.results.len(),
                    max_pages: args.max_pages,
                    completed: session_output.completed,
                    summary: if session_output.completed { Some(session_output.summary.clone()) } else { None },
                    key_findings: None,
                    sources,
                    error: None,
                };
                
                Ok(ToolResponse::new(session_output.summary, output))
            }
            
            BrowserResearchAction::List => {
                // List all sessions for this connection
                let list_output = registry
                    .list(connection_id)
                    .await
                    .map_err(McpError::Other)?;
                
                // Build display string with session info
                let display = if list_output.sessions.is_empty() {
                    format!("No active research sessions for connection {}", list_output.connection_id)
                } else {
                    let sessions_info: Vec<String> = list_output.sessions.iter()
                        .map(|s| format!(
                            "Session {}: query='{}', completed={}, results={}",
                            s.session, s.query, s.completed, s.results_count
                        ))
                        .collect();
                    format!(
                        "Active research sessions for connection {}:\n{}",
                        list_output.connection_id,
                        sessions_info.join("\n")
                    )
                };
                
                let output = BrowserResearchOutput {
                    session: args.session,
                    status: "list".to_string(),
                    query: String::new(),
                    pages_analyzed: list_output.total,
                    max_pages: 0,
                    completed: true,
                    summary: Some(display.clone()),
                    key_findings: None,
                    sources: vec![],
                    error: None,
                };
                
                Ok(ToolResponse::new(display, output))
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
                
                let message = format!("Research session {} terminated", args.session);
                let output = BrowserResearchOutput {
                    session: args.session,
                    status: "killed".to_string(),
                    query: String::new(),
                    pages_analyzed: 0,
                    max_pages: 0,
                    completed: true,
                    summary: None,
                    key_findings: None,
                    sources: vec![],
                    error: None,
                };
                
                Ok(ToolResponse::new(message, output))
            }
        }
    }
}
