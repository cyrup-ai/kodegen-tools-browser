//! Browser automation tools for AI agents
//!
//! Provides browser control, page navigation, and content extraction via chromiumoxide.

pub mod agent;
mod browser;
pub mod browser_setup;
pub mod kromekover;
mod manager;
pub mod page_enhancer;
pub mod page_extractor;
pub mod research;
mod tools;
mod utils;
pub mod web_search;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,

    #[serde(default = "default_max_steps")]
    pub max_steps: usize,

    #[serde(default = "default_search_engine")]
    pub search_engine: String,

    #[serde(default)]
    pub browser: BrowserConfig,
}

/// Browser security and launch configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    /// Run browser in headless mode
    #[serde(default = "default_headless")]
    pub headless: bool,

    /// Disable web security features (Same-Origin Policy, etc.)
    /// WARNING: Only enable for trusted content
    #[serde(default = "default_disable_security")]
    pub disable_security: bool,

    /// Window dimensions
    #[serde(default)]
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    #[serde(default = "default_window_width")]
    pub width: u32,

    #[serde(default = "default_window_height")]
    pub height: u32,
}

fn default_temperature() -> f64 {
    0.7
}
fn default_max_tokens() -> u64 {
    2048
}
fn default_max_steps() -> usize {
    10
}
fn default_search_engine() -> String {
    "google".to_string()
}

fn default_headless() -> bool {
    true
}

fn default_disable_security() -> bool {
    false  // SECURE BY DEFAULT
}

fn default_window_width() -> u32 {
    1280
}

fn default_window_height() -> u32 {
    720
}

impl Default for Config {
    fn default() -> Self {
        Self {
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            max_steps: default_max_steps(),
            search_engine: default_search_engine(),
            browser: BrowserConfig::default(),
        }
    }
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            headless: default_headless(),
            disable_security: default_disable_security(),
            window: WindowConfig::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: default_window_width(),
            height: default_window_height(),
        }
    }
}

/// Load config from config.yaml in package root
pub fn load_yaml_config() -> anyhow::Result<Config> {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config.yaml");

    if config_path.exists() {
        let contents = fs::read_to_string(&config_path)?;
        let config: Config = serde_yaml::from_str(&contents)?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

pub use browser::{
    BrowserContext, BrowserError, BrowserResult, BrowserWrapper, download_managed_browser,
    find_browser_executable, launch_browser,
};
pub use manager::BrowserManager;
pub use tools::{
    BrowserAgentTool, BrowserClickTool, BrowserExtractTextTool, BrowserNavigateTool,
    BrowserScreenshotTool, BrowserScrollTool, BrowserTypeTextTool,
    GetResearchResultTool, GetResearchStatusTool, ListResearchSessionsTool,
    StartBrowserResearchTool, StopBrowserResearchTool, WebSearchTool,
};

// Shutdown hook wrappers
struct BrowserManagerWrapper(std::sync::Arc<crate::BrowserManager>);

impl kodegen_server_http::ShutdownHook for BrowserManagerWrapper {
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let manager = self.0.clone();
        Box::pin(async move {
            manager.shutdown().await
        })
    }
}

struct ResearchSessionManagerWrapper;

impl kodegen_server_http::ShutdownHook for ResearchSessionManagerWrapper {
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            crate::research::ResearchSessionManager::global().shutdown().await
        })
    }
}

/// Start the browser tools HTTP server programmatically.
///
/// This function is designed to be called from kodegend for embedded server mode.
/// It replicates the logic from main.rs but as a library function.
///
/// # Arguments
/// * `addr` - The socket address to bind to
/// * `tls_cert` - Optional path to TLS certificate file
/// * `tls_key` - Optional path to TLS private key file
///
/// # Returns
/// Returns `Ok(())` when the server shuts down gracefully, or an error if startup/shutdown fails.
pub async fn start_server(
    addr: std::net::SocketAddr,
    tls_cert: Option<std::path::PathBuf>,
    tls_key: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    use kodegen_server_http::{Managers, RouterSet, register_tool};
    use kodegen_tools_config::ConfigManager;
    use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};
    use std::sync::Arc;

    // Initialize logging (idempotent)
    let _ = env_logger::try_init();
    
    // Initialize config
    let config = ConfigManager::new();
    config.init().await?;
    
    // Initialize tool history
    let timestamp = chrono::Utc::now();
    let pid = std::process::id();
    let instance_id = format!("{}-{}", timestamp.format("%Y%m%d-%H%M%S-%9f"), pid);
    kodegen_mcp_tool::tool_history::init_global_history(instance_id.clone()).await;
    
    // Create routers
    let mut tool_router = ToolRouter::new();
    let mut prompt_router = PromptRouter::new();
    let managers = Managers::new();
    
    // Fixed server URL for loopback tools (port 30440)
    let server_url = "http://127.0.0.1:30440/mcp".to_string();
    
    // Initialize browser manager (global singleton)
    let browser_manager = crate::BrowserManager::global();
    managers.register(BrowserManagerWrapper(browser_manager.clone())).await;
    
    // Register research session manager for shutdown
    managers.register(ResearchSessionManagerWrapper).await;
    
    // Register all 13 browser tools
    
    // Core browser automation tools (6 tools)
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::BrowserNavigateTool::new(browser_manager.clone()),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::BrowserClickTool::new(browser_manager.clone()),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::BrowserTypeTextTool::new(browser_manager.clone()),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::BrowserScreenshotTool::new(browser_manager.clone()),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::BrowserExtractTextTool::new(browser_manager.clone()),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::BrowserScrollTool::new(browser_manager.clone()),
    );

    // Advanced browser tools (1 tool)
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::BrowserAgentTool::new(browser_manager.clone(), server_url.clone()),
    );

    // Async research session tools (5 tools)
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::StartBrowserResearchTool::new(),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::GetResearchStatusTool::new(),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::GetResearchResultTool::new(),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::StopBrowserResearchTool::new(),
    );
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::ListResearchSessionsTool::new(),
    );

    // Web search tool (1 tool)
    (tool_router, prompt_router) = register_tool(
        tool_router,
        prompt_router,
        crate::WebSearchTool::new(),
    );
    
    // Create HTTP server
    let router_set = RouterSet::new(tool_router, prompt_router, managers);
    
    let session_config = rmcp::transport::streamable_http_server::session::local::SessionConfig {
        channel_capacity: 16,
        keep_alive: Some(std::time::Duration::from_secs(3600)),
    };
    let session_manager = Arc::new(
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager {
            sessions: Default::default(),
            session_config,
        }
    );
    
    let usage_tracker = kodegen_utils::usage_tracker::UsageTracker::new(
        format!("browser-{}", instance_id)
    );
    
    let server = kodegen_server_http::HttpServer::new(
        router_set.tool_router,
        router_set.prompt_router,
        usage_tracker,
        config,
        router_set.managers,
        session_manager,
    );
    
    // Start server
    let shutdown_timeout = std::time::Duration::from_secs(30);
    let tls_config = tls_cert.zip(tls_key);
    let handle = server.serve_with_tls(addr, tls_config, shutdown_timeout).await?;
    
    handle.wait_for_completion(shutdown_timeout).await
        .map_err(|e| anyhow::anyhow!("Server shutdown error: {}", e))?;
    
    Ok(())
}
