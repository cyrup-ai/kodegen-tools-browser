//! Browser instance manager for resource-efficient browser sharing
//!
//! Ensures only one browser runs at a time, shared across all tools.
//!
//! # Architecture
//!
//! Uses `Arc<Mutex<Option<BrowserWrapper>>>` pattern:
//! - Thread-safe lazy initialization via Mutex check
//! - Automatic browser launch on first use
//! - Shared access from multiple tools
//! - Proper cleanup on shutdown
//! - Health checking and automatic crash recovery
//!
//! # Async Lock Requirements
//!
//! CRITICAL: Must use `tokio::sync::Mutex`, NOT `parking_lot::RwLock`
//! - Browser operations are async (`.await` everywhere)
//! - Cannot hold sync locks across `.await` points
//! - tokio::sync::Mutex is Send-safe for async contexts
//!
//! Reference: packages/tools-citescrape/src/web_search/manager.rs

use anyhow::Result;
use chromiumoxide::page::Page;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tracing::info;

use crate::browser::{BrowserWrapper, launch_browser};

// Global singleton instance
static GLOBAL_MANAGER: OnceLock<Arc<BrowserManager>> = OnceLock::new();

/// Singleton manager for browser instances with health checking and crash recovery
///
/// Manages browser lifecycle to ensure:
/// - Only one browser instance exists at a time (lazy-loaded)
/// - Automatic launch on first use (~2-3s first call, instant after)
/// - Health checking on every access to detect crashes
/// - Automatic crash recovery (transparent to callers)
/// - Thread-safe access from multiple tools
/// - Proper cleanup when dropped or shutdown
///
/// # Performance Characteristics
///
/// - First `get_or_launch()`: ~2-3 seconds (launches Chrome)
/// - Subsequent calls (healthy browser): ~10-50ms (health check + mutex lock)
/// - Recovery from crash: ~2-3 seconds (detects + closes + re-launches)
/// - Memory: ~150MB per browser instance (Chrome process)
///
/// # Health Checking and Crash Recovery
///
/// Every call to `get_or_launch()` performs a health check via `browser.version()`
/// CDP command. If the browser has crashed, it is automatically cleaned up and
/// a new instance is launched. This provides transparent recovery without requiring
/// MCP server restarts.
///
/// # Pattern Source
///
/// Based on: packages/tools-citescrape/src/web_search/manager.rs:14-122
pub struct BrowserManager {
    browser: Arc<Mutex<Option<BrowserWrapper>>>,
    current_page: Arc<Mutex<Option<Page>>>,
}

impl BrowserManager {
    /// Get the global singleton BrowserManager instance
    ///
    /// This ensures only one browser instance runs process-wide.
    /// All tools should use this method instead of creating their own managers.
    ///
    /// # Thread Safety
    /// Uses `OnceLock` for atomic initialization - safe to call from multiple threads.
    /// First caller initializes, concurrent callers block until initialization completes.
    ///
    /// # Performance
    /// - First call: ~50ns (creates BrowserManager struct)
    /// - Subsequent calls: ~5ns (atomic pointer load)
    /// - Browser launch still lazy on first `get_or_launch()` call (~2-3s)
    ///
    /// # Example
    /// ```rust
    /// let manager = BrowserManager::global();
    /// let browser_arc = manager.get_or_launch().await?;
    /// ```
    #[must_use]
    pub fn global() -> Arc<BrowserManager> {
        GLOBAL_MANAGER
            .get_or_init(|| Arc::new(BrowserManager::new()))
            .clone()
    }

    /// Create a new BrowserManager (private - use global() instead)
    ///
    /// Browser will be lazy-loaded on first `get_or_launch()` call.
    ///
    /// This is now private to prevent accidental creation of multiple managers.
    /// External code should use `BrowserManager::global()`.
    fn new() -> Self {
        Self {
            browser: Arc::new(Mutex::new(None)),
            current_page: Arc::new(Mutex::new(None)),
        }
    }

    /// Get or launch the shared browser instance with health checking and auto-recovery
    ///
    /// # Health Check and Recovery Flow
    /// 1. Lock browser mutex
    /// 2. If browser exists, check health via version() CDP command
    /// 3. If unhealthy, close crashed browser and remove from cache
    /// 4. If no browser or was unhealthy, launch new instance
    /// 5. Return healthy browser
    ///
    /// # First Call
    /// - ~2-3s (launches browser)
    ///
    /// # Subsequent Calls (healthy browser)
    /// - <1ms (mutex lock + Arc clone)
    ///
    /// # Recovery from Crash
    /// - ~2-3s (detects crash + closes + re-launches)
    /// - Automatic, no user intervention required
    ///
    /// # Returns
    /// Arc to the browser Mutex - caller locks it to access BrowserWrapper
    ///
    /// # Example
    /// ```rust
    /// let manager = BrowserManager::global();
    /// let browser_arc = manager.get_or_launch().await?;
    /// let browser_guard = browser_arc.lock().await;
    /// if let Some(wrapper) = browser_guard.as_ref() {
    ///     let page = wrapper.browser().new_page("https://httpbin.org/html").await?;
    /// }
    /// ```
    pub async fn get_or_launch(&self) -> Result<Arc<Mutex<Option<BrowserWrapper>>>> {
        let mut guard = self.browser.lock().await;

        // Health check: if browser exists, verify it's alive
        if let Some(wrapper) = guard.as_ref() {
            match wrapper.browser().version().await {
                Ok(_) => {
                    tracing::debug!("Browser health check passed, reusing existing browser");
                    // Browser is healthy, return it
                    drop(guard); // Release lock
                    return Ok(self.browser.clone());
                }
                Err(e) => {
                    tracing::warn!("Browser health check failed: {}. Triggering recovery...", e);

                    // Take ownership and clean up crashed browser
                    if let Some(mut crashed_wrapper) = guard.take() {
                        // Best-effort cleanup (may fail if process already dead)
                        let _ = crashed_wrapper.browser_mut().close().await;
                        let _ = crashed_wrapper.browser_mut().wait().await;
                        crashed_wrapper.cleanup_temp_dir();
                    }

                    tracing::info!("Crashed browser cleaned up, launching new instance");
                }
            }
        }

        // No browser exists or previous one crashed - launch new one
        tracing::info!("Launching browser (first time or after recovery)");
        let (browser, handler, user_data_dir) = launch_browser().await?;
        let wrapper = BrowserWrapper::new(browser, handler, user_data_dir);
        *guard = Some(wrapper);
        drop(guard);

        Ok(self.browser.clone())
    }

    /// Shutdown the browser if running
    ///
    /// Explicitly closes the browser process and cleans up resources.
    /// Safe to call multiple times (subsequent calls are no-ops).
    ///
    /// # Critical Implementation Note
    ///
    /// We must call BOTH:
    /// 1. `browser.close().await` - Sends close command to Chrome
    /// 2. `browser.wait().await` - Waits for process to fully exit
    ///
    /// WHY: `BrowserWrapper::drop()` only aborts the handler task.
    /// It does NOT close the browser process. Without explicit close(),
    /// Chrome process becomes a zombie and logs warnings.
    ///
    /// # Example from citescrape
    /// ```rust
    /// // packages/tools-citescrape/src/web_search/manager.rs:99-114
    /// if let Some(mut wrapper) = browser_lock.take() {
    ///     info!("Shutting down browser");
    ///     
    ///     // 1. Close the browser
    ///     if let Err(e) = wrapper.browser_mut().close().await {
    ///         tracing::warn!("Failed to close browser cleanly: {}", e);
    ///     }
    ///     
    ///     // 2. Wait for process to fully exit
    ///     if let Err(e) = wrapper.browser_mut().wait().await {
    ///         tracing::warn!("Failed to wait for browser exit: {}", e);
    ///     }
    ///     
    ///     // 3. Now drop the wrapper (calls handler.abort())
    ///     drop(wrapper);
    /// }
    /// ```
    ///
    /// Based on: packages/tools-citescrape/src/web_search/manager.rs:88-122
    pub async fn shutdown(&self) -> Result<()> {
        let mut guard = self.browser.lock().await;

        if let Some(mut wrapper) = guard.take() {
            info!("Shutting down browser");

            // Close browser gracefully
            if let Err(e) = wrapper.browser_mut().close().await {
                tracing::warn!("Failed to close browser cleanly: {}", e);
            }

            // Wait for process to fully exit
            if let Err(e) = wrapper.browser_mut().wait().await {
                tracing::warn!("Failed to wait for browser exit: {}", e);
            }

            // Cleanup temp directory
            wrapper.cleanup_temp_dir();

            drop(wrapper);
        }

        Ok(())
    }

    /// Get the current active page, if one exists
    ///
    /// Returns the page set by the most recent navigate() call.
    /// Other browser tools (type_text, click, etc.) should use this
    /// to get the page to interact with.
    pub async fn get_current_page(&self) -> Option<Page> {
        self.current_page.lock().await.clone()
    }

    /// Set the current active page
    ///
    /// Called by navigate() to store the page for other tools to use.
    /// Replaces any previously stored page (which gets automatically dropped/closed).
    pub async fn set_current_page(&self, page: Page) {
        *self.current_page.lock().await = Some(page);
    }

    /// Check if browser is currently running
    ///
    /// Non-blocking check of browser state.
    pub async fn is_browser_running(&self) -> bool {
        self.browser.lock().await.is_some()
    }
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        // Cleanup happens via BrowserWrapper::drop() automatically
        // However, this is NOT a clean shutdown - it only aborts the handler
        // For clean shutdown, call shutdown().await before dropping
        info!("BrowserManager dropping - browser will be cleaned up");
    }
}

// ShutdownHook implementation for MCP server integration
#[cfg(feature = "server")]
use kodegen_server_http::ShutdownHook;

#[cfg(feature = "server")]
impl ShutdownHook for BrowserManager {
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            BrowserManager::shutdown(self).await
        })
    }
}

