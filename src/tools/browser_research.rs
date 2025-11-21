//! `browser_research` MCP tool implementation
//!
//! Long-running browser research with real-time progress streaming.
//! Blocks until research completes, eliminating need for polling/sessions.

use crate::utils::{DeepResearch, ResearchOptions, ResearchResult};
use kodegen_mcp_schema::browser::{BrowserResearchArgs, BrowserResearchPromptArgs, BROWSER_RESEARCH};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// =============================================================================
// OUTPUT SCHEMA
// =============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct BrowserResearchOutput {
    /// AI-generated comprehensive summary of research findings
    pub comprehensive_summary: String,

    /// List of source URLs
    pub sources: Vec<String>,

    /// Key findings (first line from each page summary)
    pub key_findings: Vec<String>,

    /// Individual page results with full details
    pub individual_results: Vec<PageResult>,

    /// Total pages successfully analyzed
    pub pages_visited: usize,

    /// Time elapsed in seconds
    pub elapsed_seconds: f64,

    /// Whether timeout was reached (partial results)
    pub timeout_reached: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageResult {
    pub url: String,
    pub title: String,
    pub summary: String,
    pub content_length: usize,
    pub timestamp: String,
}

// =============================================================================
// TOOL IMPLEMENTATION
// =============================================================================

#[derive(Clone)]
pub struct BrowserResearchTool {
    browser_manager: Arc<crate::BrowserManager>,
}

impl BrowserResearchTool {
    pub fn new(browser_manager: Arc<crate::BrowserManager>) -> Self {
        Self { browser_manager }
    }
}

impl Tool for BrowserResearchTool {
    type Args = BrowserResearchArgs;
    type PromptArgs = BrowserResearchPromptArgs;

    fn name() -> &'static str {
        BROWSER_RESEARCH
    }

    fn description() -> &'static str {
        "Perform deep web research on a query with real-time progress streaming.\n\n\
         Searches web, crawls multiple pages, extracts content, and generates AI summaries.\n\
         Blocks until complete (20-120 seconds depending on max_pages).\n\
         Streams progress notifications as each page is analyzed.\n\n\
         Returns comprehensive summary, sources, and individual page results.\n\n\
         Example: browser_research({\"query\": \"Rust async best practices\", \"max_pages\": 5})"
    }

    fn read_only() -> bool {
        false // Creates browser sessions
    }

    fn destructive() -> bool {
        false
    }

    fn idempotent() -> bool {
        false // Same query may yield different results over time
    }

    fn open_world() -> bool {
        true // Accesses external web resources
    }

    async fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> Result<Vec<Content>, McpError> {
        let start = Instant::now();
        
        // Validate query
        if args.query.trim().is_empty() {
            return Err(McpError::invalid_arguments("Research query cannot be empty"));
        }

        // Calculate timeout (max_pages * timeout_per_page + buffer)
        // Each page takes ~timeout_seconds, add 60s buffer for search/summary
        let total_timeout_secs = (args.max_pages as u64 * args.timeout_seconds) + 60;
        let timeout = Duration::from_secs(total_timeout_secs);

        // Create shared result storage (matches current architecture)
        let results: Arc<RwLock<Vec<ResearchResult>>> = Arc::new(RwLock::new(Vec::new()));
        let total_results = Arc::new(std::sync::atomic::AtomicUsize::new(0));

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

        // Clone Arc pointers for progress monitor and research task
        let results_monitor = Arc::clone(&results);
        let total_monitor = Arc::clone(&total_results);
        let results_exec = Arc::clone(&results);
        let total_exec = Arc::clone(&total_results);
        let query_clone = args.query.clone();
        let ctx_monitor = ctx.clone();
        let max_pages = args.max_pages;

        // Spawn progress monitoring task
        let monitor_cancel = tokio_util::sync::CancellationToken::new();
        let monitor_cancel_clone = monitor_cancel.clone();
        let progress_task = tokio::spawn(async move {
            let mut last_count = 0;
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let current_count = total_monitor.load(std::sync::atomic::Ordering::Acquire);
                        
                        // If new page analyzed, stream its title
                        if current_count > last_count {
                            // Read latest result
                            let results_guard = results_monitor.read().await;
                            if let Some(latest) = results_guard.last() {
                                let status = format!(
                                    "Analyzed {}/{}: {} - {}",
                                    current_count,
                                    max_pages,
                                    latest.title,
                                    latest.url
                                );
                                ctx_monitor.stream(&status).await.ok();
                            }
                            last_count = current_count;
                        }
                    }
                    _ = monitor_cancel_clone.cancelled() => {
                        break;
                    }
                }
            }
        });

        // Execute research with timeout and cancellation checking
        let research_future = research.research(&query_clone, options, results_exec, total_exec);
        
        let research_result = tokio::time::timeout(timeout, async {
            tokio::pin!(research_future);
            loop {
                // Check cancellation
                if ctx.is_cancelled() {
                    return Err(McpError::Other(anyhow::anyhow!(
                        "Research cancelled by user"
                    )));
                }

                // Execute research (non-blocking check every 100ms)
                tokio::select! {
                    result = &mut research_future => {
                        // Research completed or errored
                        return result.map_err(|e| McpError::Other(anyhow::anyhow!(
                            "Research failed: {}", e
                        )));
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        // Continue loop to check cancellation
                    }
                }
            }
        }).await;

        // Cancel progress monitor
        monitor_cancel.cancel();
        progress_task.await.ok();

        // Determine if timeout occurred
        let timeout_reached = research_result.is_err();
        
        // Build final results (even if timeout, we have partial results)
        let results_guard = results.read().await;
        let results_vec = results_guard.clone();
        drop(results_guard);

        if results_vec.is_empty() {
            return Err(McpError::Other(anyhow::anyhow!(
                "Research completed but no results available"
            )));
        }

        // Build comprehensive summary
        let pages_visited = results_vec.len();
        let mut comprehensive_summary = format!("# Research Report: {}\n\n", args.query);
        comprehensive_summary.push_str(&format!("Analyzed {} pages in {:.1}s\n\n", 
            pages_visited, start.elapsed().as_secs_f64()));

        for (i, result) in results_vec.iter().enumerate() {
            comprehensive_summary.push_str(&format!("## Source {} - {}\n", i + 1, result.title));
            comprehensive_summary.push_str(&format!("URL: {}\n\n", result.url));
            comprehensive_summary.push_str(&result.summary);
            comprehensive_summary.push_str("\n\n---\n\n");
        }

        // Extract key findings (first line of each summary)
        let key_findings: Vec<String> = results_vec
            .iter()
            .filter_map(|r| {
                let first_line = r.summary.lines().next()?;
                if !first_line.is_empty() {
                    Some(format!("{}: {}", r.title, first_line))
                } else {
                    None
                }
            })
            .collect();

        // Build sources list
        let sources: Vec<String> = results_vec.iter().map(|r| r.url.clone()).collect();

        // Build individual results
        let individual_results: Vec<PageResult> = results_vec
            .iter()
            .map(|r| PageResult {
                url: r.url.clone(),
                title: r.title.clone(),
                summary: r.summary.clone(),
                content_length: r.content.len(),
                timestamp: r.timestamp.to_rfc3339(),
            })
            .collect();

        // Create output
        let output = BrowserResearchOutput {
            comprehensive_summary,
            sources,
            key_findings,
            individual_results,
            pages_visited,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            timeout_reached,
        };

        // Serialize to JSON
        let json_str = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::Other(e.into()))?;

        Ok(vec![Content::text(json_str)])
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I perform web research?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "Use browser_research for deep web research with real-time progress:\n\n\
                     **Basic usage:**\n\
                     ```json\n\
                     browser_research({\n\
                       \"query\": \"Rust async best practices\",\n\
                       \"max_pages\": 5\n\
                     })\n\
                     ```\n\n\
                     **Response:**\n\
                     ```json\n\
                     {\n\
                       \"comprehensive_summary\": \"# Research Report: Rust async best practices\\n\\n...\",\n\
                       \"sources\": [\n\
                         \"https://rust-lang.org/async\",\n\
                         \"https://tokio.rs/tutorial\"\n\
                       ],\n\
                       \"key_findings\": [\n\
                         \"Async Programming in Rust: Use async/await for concurrent operations\",\n\
                         \"Tokio Tutorial: Tokio is the most popular async runtime\"\n\
                       ],\n\
                       \"pages_visited\": 5,\n\
                       \"elapsed_seconds\": 23.4,\n\
                       \"timeout_reached\": false\n\
                     }\n\
                     ```\n\n\
                     **Advanced options:**\n\
                     ```json\n\
                     {\n\
                       \"query\": \"WebGPU tutorial\",\n\
                       \"max_pages\": 10,\n\
                       \"max_depth\": 3,\n\
                       \"search_engine\": \"duckduckgo\",\n\
                       \"temperature\": 0.3,\n\
                       \"timeout_seconds\": 90\n\
                     }\n\
                     ```\n\n\
                     The tool blocks until complete and streams progress:\n\
                     - \"Analyzed 1/5: Async Programming - rust-lang.org\"\n\
                     - \"Analyzed 2/5: Tokio Tutorial - tokio.rs\"\n\
                     - \"Analyzed 3/5: async/await Patterns - blog.rust-lang.org\"",
                ),
            },
        ])
    }
}
