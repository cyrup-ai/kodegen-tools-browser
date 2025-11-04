//! Browser lifecycle management for web search
//!
//! Handles launching and managing chromiumoxide browser instances with
//! stealth configuration to avoid bot detection.

use anyhow::{Context, Result};
use chromiumoxide::browser::Browser;
use chromiumoxide::page::Page;
use std::path::PathBuf;
use tokio::task::JoinHandle;
use tracing::info;

/// Wrapper for Browser and its event handler task
///
/// Ensures handler is properly cleaned up when browser is dropped.
/// Handler MUST be aborted to prevent it running indefinitely after
/// browser is closed.
pub struct BrowserWrapper {
    browser: Browser,
    handler: JoinHandle<()>,
    user_data_dir: Option<PathBuf>,
}

impl BrowserWrapper {
    pub(crate) fn new(browser: Browser, handler: JoinHandle<()>, user_data_dir: PathBuf) -> Self {
        Self {
            browser,
            handler,
            user_data_dir: Some(user_data_dir),
        }
    }

    /// Get reference to inner browser
    pub(crate) fn browser(&self) -> &Browser {
        &self.browser
    }

    /// Get mutable reference to inner browser
    pub(crate) fn browser_mut(&mut self) -> &mut Browser {
        &mut self.browser
    }

    /// Clean up temp directory (blocking operation)
    ///
    /// MUST be called AFTER `browser.wait()` completes to ensure Chrome
    /// has released all file handles. Windows will fail to remove locked files.
    ///
    /// Uses blocking `std::fs::remove_dir_all()` because this may be called
    /// from Drop context where async is not available.
    ///
    /// Pattern from: forks/surrealdb/crates/language-tests/src/temp_dir.rs:55-57
    pub fn cleanup_temp_dir(&mut self) {
        if let Some(path) = self.user_data_dir.take() {
            info!("Cleaning up temp directory: {}", path.display());
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!(
                    "Failed to clean up temp directory {}: {}. Manual cleanup may be required.",
                    path.display(),
                    e
                );
            }
        }
    }

    /// Prevent automatic cleanup (for debugging)
    ///
    /// Useful when investigating Chrome crashes - preserves profile for inspection
    #[allow(dead_code)]
    pub fn keep_temp_dir(mut self) {
        self.user_data_dir = None;
    }
}

impl Drop for BrowserWrapper {
    fn drop(&mut self) {
        info!("Dropping BrowserWrapper - aborting handler task");
        self.handler.abort();
        // Handler will be awaited/cleaned up by tokio runtime
        // Browser::drop() will automatically kill the Chrome process

        // Warn if temp directory was not cleaned up via proper shutdown path
        if self.user_data_dir.is_some() {
            tracing::warn!(
                "BrowserWrapper dropped without explicit cleanup. \
                Temp directory will be orphaned: {}. \
                Call BrowserManager::shutdown() before dropping to ensure proper cleanup.",
                self.user_data_dir.as_ref().unwrap().display()
            );
        }
    }
}

/// Launch a new browser instance with stealth configuration
///
/// Returns tuple of (Browser, JoinHandle, PathBuf) where PathBuf is the
/// temp directory that MUST be cleaned up after browser shuts down.
///
/// Uses shared `browser_setup::launch_browser` with unique profile directory
/// to prevent Chrome profile lock contention when multiple browser instances run.
///
/// # Handler Lifecycle
/// The returned `JoinHandle` MUST be aborted when done to stop the browser process.
/// `BrowserWrapper::drop()` handles this automatically.
pub async fn launch_browser() -> Result<(Browser, JoinHandle<()>, PathBuf)> {
    info!("Launching main browser instance");

    // Load configuration
    let config = crate::load_yaml_config().unwrap_or_default();

    // Create unique temp directory for main browser (prevents profile lock with web_search)
    let user_data_dir = std::env::temp_dir().join(format!("kodegen_browser_main_{}", std::process::id()));

    // Use shared browser launcher with profile isolation
    // Pattern from: packages/tools-citescrape/src/browser_setup.rs:209-296
    let (browser, handler) = crate::browser_setup::launch_browser(
        config.browser.headless,
        Some(user_data_dir.clone()),
        config.browser.disable_security,
    ).await?;

    Ok((browser, handler, user_data_dir))
}

/// Create a blank page for stealth injection
///
/// Creates a page with about:blank URL, which is required for proper
/// stealth injection timing. The page must be blank before
/// stealth features are applied, then navigation to the target URL occurs.
///
/// # Arguments
/// * `wrapper` - `BrowserWrapper` containing the browser instance
///
/// # Returns
/// A blank Page instance ready for stealth enhancement
///
/// # Based on
/// - packages/citescrape/src/crawl_engine/core.rs:231-237 (about:blank pattern)
pub async fn create_blank_page(wrapper: &BrowserWrapper) -> Result<Page> {
    let page = wrapper
        .browser()
        .new_page("about:blank")
        .await
        .context("Failed to create blank page")?;

    info!("Created blank page for stealth injection");
    Ok(page)
}

/// Get the current/active page from the browser
///
/// Uses chromiumoxide's built-in page tracking to retrieve the first page.
/// This should be called after browser_navigate has created and navigated a page.
///
/// # Arguments
/// * `wrapper` - `BrowserWrapper` containing the browser instance
///
/// # Returns
/// The first/primary Page instance
///
/// # Errors
/// Returns error if no pages exist (user must call browser_navigate first)
///
/// # Based on
/// - tmp/chromiumoxide/src/browser.rs:524-531 (browser.pages() API)
pub async fn get_current_page(wrapper: &BrowserWrapper) -> Result<Page> {
    let pages = wrapper
        .browser()
        .pages()
        .await
        .context("Failed to get browser pages")?;

    pages
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No page loaded. Call browser_navigate first."))
}
