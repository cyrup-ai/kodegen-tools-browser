//! Deep research module - infrastructure for future use

use std::sync::Arc;

// Workspace LLM infrastructure
use kodegen_candle_agent::prelude::*;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use tokio::task::JoinSet;
use tokio::sync::Semaphore;

use crate::utils::errors::UtilsError;

// Browser tool imports for direct library integration
use kodegen_mcp_schema::browser::BrowserNavigateArgs;
use crate::tools::BrowserNavigateTool;

// Page metadata extraction
use crate::page_extractor::{PageMetadata, extract_page_info};

/// Research result containing extracted information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    pub url: String,
    pub title: String,
    pub content: String,
    pub summary: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub metadata: PageMetadata,
}

/// Research options for izing research behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchOptions {
    pub max_pages: usize,
    pub max_depth: usize,
    pub search_engine: String,
    pub include_links: bool,
    pub extract_tables: bool,
    pub extract_images: bool,
    pub timeout_seconds: u64,
}

impl Default for ResearchOptions {
    fn default() -> Self {
        Self {
            max_pages: 5,
            max_depth: 2,
            search_engine: "google".to_string(),
            include_links: true,
            extract_tables: true,
            extract_images: false,
            timeout_seconds: 60,
        }
    }
}

/// Deep research service using direct library integration
///
/// All browser operations call local functions directly:
/// - web_search (local) - DuckDuckGo search via global BrowserManager
/// - browser_navigate - URL loading via BrowserNavigateTool (library call)
/// - browser_extract_text - Content extraction via BrowserExtractTextTool (library call)
///
/// LLM operations use CandleFluentAi streaming (no trait objects).
#[derive(Clone)]
pub struct DeepResearch {
    /// Browser manager for all browser operations
    browser_manager: Arc<crate::BrowserManager>,

    /// LLM temperature for summarization (0.0 = deterministic, 2.0 = creative)
    temperature: f64,

    /// Maximum tokens for LLM generation
    max_tokens: u64,

    /// Track visited URLs to avoid duplicates
    visited_urls: Arc<Mutex<Vec<String>>>,
}

impl DeepResearch {
    /// Create new DeepResearch instance
    ///
    /// # Arguments
    /// * `browser_manager` - Global browser manager for all browser operations
    /// * `temperature` - LLM sampling temperature (0.0-2.0)
    /// * `max_tokens` - Maximum tokens for LLM generation
    pub fn new(
        browser_manager: Arc<crate::BrowserManager>,
        temperature: f64,
        max_tokens: u64,
    ) -> Self {
        Self {
            browser_manager,
            temperature,
            max_tokens,
            visited_urls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Perform web research on a query (incremental streaming pattern)
    pub async fn research(
        &self,
        query: &str,
        options: Option<ResearchOptions>,
        results: Arc<tokio::sync::RwLock<Vec<ResearchResult>>>,
        total_results: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), UtilsError> {
        let options = options.unwrap_or_default();

        // Reset visited URLs
        let mut visited = self.visited_urls.lock().await;
        visited.clear();
        drop(visited);

        // Search for query
        let search_results = self.search_query(query, &options).await?;

        // Process each search result in parallel with semaphore-controlled concurrency
        let semaphore = Arc::new(Semaphore::new(3)); // 3 concurrent URLs max
        let mut join_set = JoinSet::new();

        // Spawn parallel task for each URL
        for url in search_results.iter().take(options.max_pages) {
            let url = url.clone();
            let options = options.clone();
            let results = Arc::clone(&results);
            let total_results = Arc::clone(&total_results);
            let semaphore = Arc::clone(&semaphore);
            let research = self.clone(); // All fields are Clone via Arc or Copy

            join_set.spawn(async move {
                // Acquire semaphore permit (blocks if 3 tasks already running)
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|e| UtilsError::UnexpectedError(format!("Semaphore error: {}", e)))?;

                // Process URL (duplicate checking now atomic via Change 3)
                match research.process_url(&url, &options).await {
                    Ok(result) => {
                        // Append result immediately (incremental streaming - UNCHANGED)
                        {
                            let mut results_guard = results.write().await;
                            results_guard.push(result);
                        }
                        // Update counter atomically (UNCHANGED)
                        total_results.fetch_add(1, std::sync::atomic::Ordering::Release);
                        Ok(())
                    }
                    Err(e) => {
                        // Log error and continue (UNCHANGED behavior)
                        warn!("Error processing URL {}: {}", url, e);
                        Err(e)
                    }
                }
                // Semaphore permit automatically released when _permit drops
            });
        }

        // Wait for all parallel tasks to complete
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(())) => {
                    // URL processed successfully
                }
                Ok(Err(_e)) => {
                    // URL processing error (already logged in task)
                }
                Err(e) => {
                    // Task panic - log it
                    warn!("Research task panicked: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Search for query using web_search module directly
    ///
    /// Calls local web_search which provides DuckDuckGo search
    /// with kromekover stealth, retries, and structured result parsing.
    ///
    /// # Arguments
    /// * `query` - Search query string
    /// * `options` - Research options (currently unused, web_search has sensible defaults)
    ///
    /// # Returns
    /// Vector of URLs from search results (up to 10)
    ///
    /// # Direct Integration
    /// This method calls web_search directly (same package) instead of via MCP.
    /// Benefits:
    /// - Faster (no IPC overhead)
    /// - Simpler (no serialization/deserialization)
    /// - More reliable (no network/process dependencies)
    async fn search_query(
        &self,
        query: &str,
        _options: &ResearchOptions,
    ) -> Result<Vec<String>, UtilsError> {
        debug!("Searching DuckDuckGo via web_search (direct): {}", query);

        // Call web_search directly (same package, no MCP needed)
        let search_results = crate::web_search::search_with_manager(&self.browser_manager, query)
            .await
            .map_err(|e| UtilsError::BrowserError(e.to_string()))?;

        // Extract URLs from SearchResults
        let urls: Vec<String> = search_results.results.iter()
            .map(|r| r.url.clone())
            .collect();

        if urls.is_empty() {
            warn!("web_search returned no results for query: {}", query);
        } else {
            info!("web_search found {} URLs for query: {}", urls.len(), query);
        }

        Ok(urls)
    }

    /// Process a URL and extract content
    async fn process_url(
        &self,
        url: &str,
        options: &ResearchOptions,
    ) -> Result<ResearchResult, UtilsError> {
        // Check if already visited and mark atomically (prevents race conditions)
        {
            let mut visited = self.visited_urls.lock().await;
            if visited.contains(&url.to_string()) {
                return Err(UtilsError::UnexpectedError("URL already visited".into()));
            }
            // Mark as visited immediately (atomic with check)
            visited.push(url.to_string());
        } // Lock released here

        // 1. NAVIGATE AND CAPTURE PAGE HANDLE
        debug!("Navigating to {} and capturing page handle", url);
        
        let nav_tool = BrowserNavigateTool::new(self.browser_manager.clone());
        let nav_args = BrowserNavigateArgs {
            url: url.to_string(),
            wait_for_selector: None,
            timeout_ms: Some(options.timeout_seconds * 1000),
        };
        
        // Call internal method to get BOTH page and result
        let (page, nav_result) = nav_tool
            .navigate_and_capture_page(nav_args)
            .await
            .map_err(|e| UtilsError::BrowserError(e.to_string()))?;

        // Parse final URL from result
        let final_url = nav_result
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or(url)
            .to_string();

        // 2. EXTRACT PAGE INFO - uses captured page
        debug!("Extracting page info from captured page");
        let page_info = extract_page_info(page.clone())
            .await
            .map_err(|e| UtilsError::BrowserError(e.to_string()))?;

        let title = page_info.title;

        // 3. EXTRACT CONTENT DIRECTLY FROM CAPTURED PAGE
        debug!("Extracting content from captured page");
        
        // Extract text using JavaScript evaluation on captured page
        // This ensures we extract from the correct page in parallel execution
        let eval_result = page
            .evaluate("document.body.innerText")
            .await
            .map_err(|e| UtilsError::BrowserError(format!("Failed to extract text: {}", e)))?;

        // Parse result value
        let text_value = eval_result
            .into_value()
            .map_err(|e| UtilsError::BrowserError(format!("Failed to parse text result: {}", e)))?;

        // Extract string from Value, with fallback for SPAs
        let content = if let serde_json::Value::String(text) = text_value {
            text
        } else {
            // Fallback: get HTML and convert to text (for SPAs where innerText is empty)
            let html = page
                .content()
                .await
                .map_err(|e| UtilsError::BrowserError(format!("Failed to get HTML content: {}", e)))?;
            html2md::parse_html(&html)
        };

        // 4. GENERATE SUMMARY WITH CANDLEFLUENTAI
        let summary = self.summarize_content(&title, &content).await?;

        Ok(ResearchResult {
            url: final_url,
            title,
            content,
            summary,
            timestamp: chrono::Utc::now(),
            metadata: page_info.metadata,
        })
    }

    /// Summarize content using CandleFluentAi streaming
    ///
    /// Creates an LLM agent on-demand with configured temperature and max_tokens.
    /// Streams response in real-time for better perceived performance.
    ///
    /// # Pattern Reference
    /// Based on: packages/tools-candle-agent/examples/fluent_builder.rs:58-90
    async fn summarize_content(&self, title: &str, content: &str) -> Result<String, UtilsError> {
        // Truncate content if too long (avoid context overflow)
        // Use char-based truncation to prevent UTF-8 boundary panics
        let max_content_chars = 8000;
        let truncated_content = if content.chars().count() > max_content_chars {
            let truncated: String = content.chars().take(max_content_chars).collect();
            format!("{}... [content truncated]", truncated)
        } else {
            content.to_string()
        };

        // Build prompt
        let prompt = format!(
            "Please summarize the following webpage content.\n\nTitle: '{}'\n\nContent:\n{}",
            title, truncated_content
        );

        // Create streaming agent with CandleFluentAi builder
        let mut stream = CandleFluentAi::agent_role("research-summarizer")
            .temperature(self.temperature)
            .max_tokens(self.max_tokens)
            .system_prompt(
                "You are an AI research assistant that summarizes web content accurately \
                and concisely. Extract key information, findings, data points, and conclusions. \
                Organize information logically and provide accurate section headers where appropriate. \
                Focus on factual content, avoid speculation."
            )
            .on_chunk(|chunk| async move {
                // Pass through chunks (could add logging here)
                chunk
            })
            .into_agent()
            .map_err(|e| UtilsError::AgentError(e.to_string()))?
            .chat(move |_conversation| {
                let prompt_clone = prompt.clone();
                async move { CandleChatLoop::UserPrompt(prompt_clone) }
            })
            .map_err(|e| UtilsError::LlmError(e.to_string()))?;

        // Collect streamed response into String
        use tokio_stream::StreamExt;
        // Pre-allocate for research summary streaming
        // Typical summaries: 1000-2000 tokens (~4-8KB)
        // Use 8KB (8192 bytes) conservative estimate
        let mut summary = String::with_capacity(8192);
        while let Some(chunk) = stream.next().await {
            match chunk {
                CandleMessageChunk::Text(text) => {
                    summary.push_str(&text);
                }
                CandleMessageChunk::Complete { .. } => {
                    // Generation complete, summary is ready
                    break;
                }
                _ => {
                    // Ignore other chunk types (Thinking, etc.)
                }
            }
        }

        if summary.is_empty() {
            return Err(UtilsError::LlmError("Empty summary generated".into()));
        }

        Ok(summary)
    }
}
