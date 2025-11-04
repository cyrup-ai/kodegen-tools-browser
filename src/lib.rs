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
