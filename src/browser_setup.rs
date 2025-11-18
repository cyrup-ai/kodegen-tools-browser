use anyhow::{Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfigBuilder, HeadlessMode};
use chromiumoxide::fetcher::{BrowserFetcher, BrowserFetcherOptions};
use futures::StreamExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::task::{self, JoinHandle};
use tracing::{error, info, trace, warn};

use crate::utils::constants::CHROME_USER_AGENT;

/// RAII guard for temporary directory cleanup
///
/// Automatically removes the directory on drop unless consumed by `into_path()`.
/// This ensures cleanup happens on all error paths without manual intervention.
///
/// Pattern based on: src/browser/wrapper.rs:74-89 (BrowserWrapper::Drop)
struct TempDirGuard {
    path: PathBuf,
    keep: bool,
}

impl TempDirGuard {
    /// Create directory and guard for automatic cleanup
    fn new(path: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&path)
            .context("Failed to create user data directory")?;
        Ok(Self { 
            path, 
            keep: false 
        })
    }

    /// Consume guard and return path, preventing automatic cleanup
    ///
    /// Call this on success to transfer ownership to BrowserWrapper
    fn into_path(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.keep {
            if let Err(e) = std::fs::remove_dir_all(&self.path) {
                warn!("Failed to clean up temp dir {}: {}", 
                    self.path.display(), e);
            } else {
                info!("Cleaned up temp dir after launch failure: {}", 
                    self.path.display());
            }
        }
    }
}

/// Find Chrome/Chromium executable on the system with platform-specific search paths.
pub async fn find_browser_executable() -> Result<PathBuf> {
    // First check environment variable which overrides all other methods
    if let Ok(path) = std::env::var("CHROMIUM_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            info!(
                "Using browser from CHROMIUM_PATH environment variable: {}",
                path.display()
            );
            return Ok(path);
        }
        warn!(
            "CHROMIUM_PATH environment variable points to non-existent file: {}",
            path.display()
        );
    }

    // Common Chrome/Chromium installation paths by platform
    let paths = if cfg!(target_os = "windows") {
        vec![
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"%PROGRAMFILES%\Google\Chrome\Application\chrome.exe",
            r"%PROGRAMFILES(X86)%\Google\Chrome\Application\chrome.exe",
            r"%LOCALAPPDATA%\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Chromium\Application\chrome.exe",
            r"C:\Program Files (x86)\Chromium\Application\chrome.exe",
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            "/Applications/Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "~/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "~/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/opt/homebrew/bin/chromium",
        ]
    } else {
        // Linux
        vec![
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
            "/usr/local/bin/chromium",
            "/opt/google/chrome/chrome",
        ]
    };

    // Try each path
    for path_str in paths {
        let path = if path_str.starts_with('~') {
            // Expand home directory if path starts with ~
            if let Some(home) = dirs::home_dir() {
                home.join(&path_str[2..])
            } else {
                continue;
            }
        } else if path_str.contains('%') && cfg!(target_os = "windows") {
            // Expand environment variables on Windows (%VAR% tokens)
            let expanded = expand_windows_env_vars(path_str);
            PathBuf::from(expanded)
        } else {
            PathBuf::from(path_str)
        };

        if path.exists() {
            info!("Found browser at: {}", path.display());
            return Ok(path);
        }
    }

    // Use 'which' command to find Chromium on Unix systems
    if !cfg!(target_os = "windows") {
        for cmd in &["chromium", "chromium-browser", "google-chrome", "chrome"] {
            let output = Command::new("which").arg(cmd).output();

            if let Ok(output) = output
                && output.status.success()
            {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    let path = PathBuf::from(path_str);
                    info!("Found browser using 'which' command: {}", path.display());
                    return Ok(path);
                }
            }
        }
    }

    // No browser found, inform user
    warn!("No Chrome/Chromium executable found. Will download and use fetcher.");
    Err(anyhow::anyhow!("Chrome/Chromium executable not found"))
}

/// Expand Windows environment variables in the form %VAR% within a path string.
///
/// Iterates through the string, finding %VAR% patterns and replacing them with
/// their environment variable values. If a variable doesn't exist, the original
/// %VAR% token is preserved.
///
/// # Arguments
/// * `path` - Path string potentially containing %VAR% tokens
///
/// # Returns
/// Expanded path string with all available environment variables substituted
///
/// # Example
/// ```
/// let path = "%PROGRAMFILES%\\Google\\Chrome\\Application\\chrome.exe";
/// let expanded = expand_windows_env_vars(path);
/// // Result: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"
/// ```
fn expand_windows_env_vars(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            // Found start of potential environment variable
            let var_name: String = chars.by_ref().take_while(|&c| c != '%').collect();

            if !var_name.is_empty() {
                // Try to expand the variable
                if let Ok(value) = std::env::var(&var_name) {
                    result.push_str(&value);
                } else {
                    // Variable not found, preserve original %VAR% token
                    result.push('%');
                    result.push_str(&var_name);
                    result.push('%');
                }
            } else {
                // Empty %% sequence, keep single %
                result.push('%');
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Downloads and manages Chromium browser if not found locally.
/// Returns a path to the downloaded executable.
pub async fn download_managed_browser() -> Result<PathBuf> {
    info!("Downloading managed Chromium browser...");

    // Create cache directory for downloaded browser
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| {
            let fallback = std::env::temp_dir().join(".cache");
            warn!(
                "Could not determine system cache directory, using temp directory fallback: {}",
                fallback.display()
            );
            fallback
        })
        .join("enigo/chromium");

    std::fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;

    // Use fetcher to download Chrome
    let fetcher = BrowserFetcher::new(
        BrowserFetcherOptions::builder()
            .with_path(&cache_dir)
            .build()
            .context("Failed to build fetcher options")?,
    );

    // Download Chrome
    let revision_info = fetcher.fetch().await.context("Failed to fetch browser")?;

    info!(
        "Downloaded Chromium to: {}",
        revision_info.folder_path.display()
    );

    Ok(revision_info.executable_path)
}

/// Unified browser launcher that finds or downloads Chrome/Chromium and
/// configures it with stealth mode settings.
///
/// # Arguments
/// * `headless` - Whether to run browser in headless mode
/// * `chrome_data_dir` - Optional custom user data directory path. If None, uses process ID fallback.
/// * `disable_security` - Whether to disable browser security features (WARNING: only for trusted content)
///
/// # Profile Isolation
/// When `chrome_data_dir` is provided, each browser instance uses a unique profile directory,
/// preventing lock contention in long-running servers with concurrent crawls.
pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
    disable_security: bool,
) -> Result<(Browser, JoinHandle<()>)> {
    // First try to find the browser
    let chrome_path = match find_browser_executable().await {
        Ok(path) => path,
        Err(_) => {
            // If not found, download a managed browser
            download_managed_browser().await?
        }
    };

    // Use provided chrome_data_dir or fall back to process ID
    let user_data_dir_path = chrome_data_dir.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("enigo_chrome_{}", std::process::id()))
    });

    // Create directory with automatic cleanup on error
    let temp_guard = TempDirGuard::new(user_data_dir_path)?;
    let user_data_dir = temp_guard.path.clone();

    // Build browser config with the executable path
    let mut config_builder = BrowserConfigBuilder::default()
        .request_timeout(Duration::from_secs(30))
        .window_size(1920, 1080)
        .user_data_dir(user_data_dir)
        .chrome_executable(chrome_path);

    // Set headless mode based on parameter
    if headless {
        config_builder = config_builder.headless_mode(HeadlessMode::default());
    } else {
        config_builder = config_builder.with_head();
    }

    // Add stealth mode arguments (benign flags always added)
    config_builder = config_builder
        .arg(format!("--user-agent={}", CHROME_USER_AGENT))
        .arg("--disable-blink-features=AutomationControlled")
        .arg("--disable-infobars")
        .arg("--disable-notifications")
        .arg("--disable-print-preview")
        .arg("--disable-desktop-notifications")
        .arg("--disable-software-rasterizer")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--enable-features=NetworkService,NetworkServiceInProcess")
        // Additional stealth arguments (benign)
        .arg("--disable-extensions")
        .arg("--disable-popup-blocking")
        .arg("--disable-background-networking")
        .arg("--disable-background-timer-throttling")
        .arg("--disable-backgrounding-occluded-windows")
        .arg("--disable-breakpad")
        .arg("--disable-component-extensions-with-background-pages")
        .arg("--disable-features=TranslateUI")
        .arg("--disable-hang-monitor")
        .arg("--disable-ipc-flooding-protection")
        .arg("--disable-prompt-on-repost")
        .arg("--metrics-recording-only")
        .arg("--password-store=basic")
        .arg("--use-mock-keychain")
        .arg("--hide-scrollbars")
        .arg("--mute-audio");

    // Conditionally add security-disabling flags
    if disable_security {
        info!("WARNING: Disabling browser security features (disable_security=true)");
        config_builder = config_builder
            .arg("--disable-web-security")
            .arg("--disable-features=IsolateOrigins,site-per-process")
            .arg("--ignore-certificate-errors");
    }

    // Always disable sandbox in containerized environments (Docker detection)
    if should_disable_sandbox() {
        info!("Detected containerized environment, disabling sandbox");
        config_builder = config_builder
            .arg("--no-sandbox")
            .arg("--disable-setuid-sandbox");
    } else if disable_security {
        // Only disable sandbox if explicitly requested AND not in container
        config_builder = config_builder
            .arg("--no-sandbox")
            .arg("--disable-setuid-sandbox");
    }

    let browser_config = config_builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build browser config: {e}"))?;

    info!("Launching browser with config: {:?}", browser_config);
    let (browser, mut handler) = Browser::launch(browser_config)
        .await
        .context("Failed to launch browser")?;

    let handler_task = task::spawn(async move {
        while let Some(h) = handler.next().await {
            if let Err(e) = h {
                let error_msg = e.to_string();
                
                // Filter out known non-fatal CDP serialization errors
                // These occur when Chrome sends CDP events that chromiumoxide doesn't recognize
                // Reference: https://github.com/mattsse/chromiumoxide/issues/167
                //            https://github.com/mattsse/chromiumoxide/issues/229
                let is_benign_serialization_error = 
                    error_msg.contains("data did not match any variant of untagged enum Message")
                    || error_msg.contains("Failed to deserialize WS response");
                
                if !is_benign_serialization_error {
                    // Log genuine errors that need attention
                    error!("Browser handler error: {:?}", e);
                } else {
                    // Optionally log at trace level for deep debugging
                    trace!("Suppressed benign CDP serialization error: {}", error_msg);
                }
            }
        }
        info!("Browser handler task completed");
    });

    // Success - prevent automatic cleanup (BrowserWrapper now owns the directory)
    temp_guard.into_path();
    
    Ok((browser, handler_task))
}

/// Detect if running in containerized environment (Docker, etc.)
/// In containers, sandbox must be disabled as setuid doesn't work
fn should_disable_sandbox() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::env::var("container").is_ok()
        || std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
}
