//! Browser automation tools for AI agents
//!
//! Provides browser control, page navigation, and content extraction via chromiumoxide.

pub mod agent;
mod browser;
pub mod browser_setup;
pub mod research;
pub mod kromekover;
mod manager;
pub mod page_enhancer;
pub mod page_extractor;
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
    BrowserResearchTool, BrowserScreenshotTool, BrowserScrollTool, BrowserTypeTextTool,
    BrowserWebSearchTool,
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

/// Start the browser tools HTTP server programmatically
///
/// Returns a ServerHandle for graceful shutdown control.
/// This function is non-blocking - the server runs in background tasks.
///
/// # Arguments
/// * `addr` - Socket address to bind to
/// * `tls_cert` - Optional path to TLS certificate file
/// * `tls_key` - Optional path to TLS private key file
///
/// # Returns
/// ServerHandle for graceful shutdown, or error if startup fails
pub async fn start_server(
    addr: std::net::SocketAddr,
    tls_cert: Option<std::path::PathBuf>,
    tls_key: Option<std::path::PathBuf>,
) -> anyhow::Result<kodegen_server_http::ServerHandle> {
    // Bind to the address first
    let listener = tokio::net::TcpListener::bind(addr).await
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", addr, e))?;

    // Convert separate cert/key into Option<(cert, key)> tuple
    let tls_config = match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => Some((cert, key)),
        _ => None,
    };

    // Delegate to start_server_with_listener
    start_server_with_listener(listener, tls_config).await
}

/// Start browser tools HTTP server using pre-bound listener (TOCTOU-safe)
///
/// This variant is used by kodegend to eliminate TOCTOU race conditions
/// during port cleanup. The listener is already bound to a port.
///
/// # Arguments
/// * `listener` - Pre-bound TcpListener (port already reserved)
/// * `tls_config` - Optional (cert_path, key_path) for HTTPS
///
/// # Returns
/// ServerHandle for graceful shutdown, or error if startup fails
pub async fn start_server_with_listener(
    listener: tokio::net::TcpListener,
    tls_config: Option<(std::path::PathBuf, std::path::PathBuf)>,
) -> anyhow::Result<kodegen_server_http::ServerHandle> {
    use kodegen_server_http::{ServerBuilder, Managers, RouterSet, register_tool};
    use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};

    let mut builder = ServerBuilder::new()
        .category(kodegen_config::CATEGORY_BROWSER)
        .register_tools(|| async {
            let mut tool_router = ToolRouter::new();
            let mut prompt_router = PromptRouter::new();
            let managers = Managers::new();

            // Fixed server URL for loopback tools
            let server_url = format!("http://127.0.0.1:{}/mcp", kodegen_config::PORT_BROWSER);

            // Initialize browser manager (global singleton)
            let browser_manager = crate::BrowserManager::global();
            managers.register(BrowserManagerWrapper(browser_manager.clone())).await;

            // Register all 9 browser tools (was 13)

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

            // Browser research tool (1 tool - replaces 5 polling tools)
            (tool_router, prompt_router) = register_tool(
                tool_router,
                prompt_router,
                crate::BrowserResearchTool::new(browser_manager.clone()),
            );

            // Web search tool (1 tool)
            (tool_router, prompt_router) = register_tool(
                tool_router,
                prompt_router,
                crate::BrowserWebSearchTool::new(),
            );

            Ok(RouterSet::new(tool_router, prompt_router, managers))
        })
        .with_listener(listener);

    if let Some((cert, key)) = tls_config {
        builder = builder.with_tls_config(cert, key);
    }

    builder.serve().await
}
