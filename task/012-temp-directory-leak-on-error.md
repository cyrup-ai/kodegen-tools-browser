# Temp Directory Leak on Error Paths

## Priority: MEDIUM

## Location
- `src/browser_setup.rs:209-297`
- `src/browser/wrapper.rs:102-116`

## Issue Description
When browser launch fails after the temp directory is created, the directory is not cleaned up. This can lead to accumulation of orphaned Chrome profile directories in the temp folder over time.

## Code Reference

### Issue in launch_browser
```rust
pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
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
    let user_data_dir = chrome_data_dir.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("enigo_chrome_{}", std::process::id()))
    });

    // Create temp directory
    std::fs::create_dir_all(&user_data_dir).context("Failed to create user data directory")?;
    // ^^^ Line 227 - Directory created

    // Build browser config with the executable path
    let mut config_builder = BrowserConfigBuilder::default()
        .request_timeout(Duration::from_secs(30))
        .window_size(1920, 1080)
        .user_data_dir(user_data_dir)  // Directory set in config
        .chrome_executable(chrome_path);

    // ... config setup ...

    let browser_config = config_builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build browser config: {e}"))?;
    // ^^^ Line 280 - If this fails, directory leaked!

    info!("Launching browser with config: {:?}", browser_config);
    let (browser, mut handler) = Browser::launch(browser_config)
        .await
        .context("Failed to launch browser")?;
    // ^^^ Line 285 - If this fails, directory leaked!

    // ... handler task setup ...

    Ok((browser, handler_task))
}
```

**Problem**: If any of these fail after line 227, the temp directory is created but never cleaned up:
1. `browser_config.build()` fails (line 278-280)
2. `Browser::launch()` fails (line 283-285)
3. Handler setup fails (less likely)

### Issue in wrapper launch_browser
```rust
pub async fn launch_browser() -> Result<(Browser, JoinHandle<()>, PathBuf)> {
    info!("Launching main browser instance");

    // Create unique temp directory for main browser (prevents profile lock with web_search)
    let user_data_dir = std::env::temp_dir().join(format!("kodegen_browser_main_{}", std::process::id()));
    // ^^^ Line 106 - Path generated but directory not created yet

    // Use shared browser launcher with profile isolation
    let (browser, handler) = crate::browser_setup::launch_browser(
        true, // headless
        Some(user_data_dir.clone())
    ).await?;
    // ^^^ Line 113 - If launch fails, directory created inside but not tracked for cleanup

    Ok((browser, handler, user_data_dir))
}
```

## Problem Details

### Failure Scenarios

**Scenario 1: Chrome Binary Not Executable**
```
1. find_browser_executable() succeeds (finds Chrome)
2. create_dir_all() succeeds (temp dir created)  ← LEAK POINT
3. Browser::launch() fails (Chrome binary lacks execute permission)
4. Error returned, temp directory never cleaned up
```

**Scenario 2: Port/Socket Conflict**
```
1. Chrome binary found
2. Temp dir created  ← LEAK POINT
3. Browser tries to bind to debugging port
4. Port already in use
5. Launch fails, temp dir leaked
```

**Scenario 3: Chrome Crashes on Startup**
```
1. Setup succeeds
2. Temp dir created  ← LEAK POINT
3. Chrome launches but crashes immediately
4. chromiumoxide detects crash
5. Returns error, temp dir leaked
```

## Impact on Production

### Severity: Medium

**Resource Impact**:
- Each leaked directory: ~10-50 MB (Chrome profile data)
- Frequency: Every failed browser launch
- Accumulation: Grows over time in long-running servers

### Real-World Numbers

**Failure Rate Assumptions**:
- 99% success rate = 1% failures
- 1000 browser launches per day = 10 failures
- 30 MB per failed directory

**Accumulation**:
- Per day: 10 × 30 MB = 300 MB
- Per week: 2.1 GB
- Per month: 9 GB

In production servers that run for months, this can become significant.

### Where Failures Happen

**Development/Testing**: High failure rate
- Misconfigured environments
- Missing Chrome installations
- Permission issues
- Port conflicts in parallel tests

**Production**: Low failure rate
- But runs longer (months)
- Still accumulates over time

## Current Cleanup Mechanisms

### Success Path ✅
```rust
// BrowserManager::shutdown() (correct)
wrapper.browser_mut().wait().await?;  // Wait for Chrome to exit
wrapper.cleanup_temp_dir();            // Clean up directory
```

### Error Path ❌
```rust
// launch_browser() fails
// Returns Err(...)
// Temp directory never passed to BrowserWrapper
// No cleanup code runs
```

## Recommended Fixes

### Option 1: RAII Guard (Recommended)

Create a guard that cleans up on drop:

```rust
struct TempDirGuard {
    path: PathBuf,
    keep: bool,
}

impl TempDirGuard {
    fn new(path: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&path)?;
        Ok(Self { path, keep: false })
    }

    fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.keep {
            if let Err(e) = std::fs::remove_dir_all(&self.path) {
                warn!("Failed to clean up temp dir {}: {}", self.path.display(), e);
            }
        }
    }
}

pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
) -> Result<(Browser, JoinHandle<()>)> {
    let chrome_path = /* ... */;

    let user_data_dir = /* ... */;

    // Create directory with cleanup guard
    let temp_guard = TempDirGuard::new(user_data_dir.clone())?;

    // Build config (may fail)
    let browser_config = config_builder.build()
        .map_err(|e| anyhow::anyhow!("Failed to build browser config: {e}"))?;

    // Launch browser (may fail)
    let (browser, handler) = Browser::launch(browser_config).await
        .context("Failed to launch browser")?;

    // Success - transfer ownership to BrowserWrapper
    temp_guard.keep();  // Prevent cleanup on drop

    Ok((browser, handler_task))
}
```

**Benefits**:
- Automatic cleanup on any error
- No manual error handling needed
- Rust idiom (RAII)

### Option 2: Manual Cleanup on Error

Add explicit error handling:

```rust
pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
) -> Result<(Browser, JoinHandle<()>)> {
    let chrome_path = /* ... */;
    let user_data_dir = /* ... */;

    std::fs::create_dir_all(&user_data_dir)
        .context("Failed to create user data directory")?;

    // Try to launch browser
    let result = async {
        let browser_config = config_builder.build()
            .map_err(|e| anyhow::anyhow!("Failed to build browser config: {e}"))?;

        let (browser, handler) = Browser::launch(browser_config).await
            .context("Failed to launch browser")?;

        Ok((browser, handler))
    }.await;

    // Clean up on error
    if result.is_err() {
        if let Err(e) = std::fs::remove_dir_all(&user_data_dir) {
            warn!("Failed to clean up temp dir after launch failure: {}", e);
        }
    }

    result
}
```

**Benefits**:
- Explicit and clear
- No new types needed

**Drawbacks**:
- More verbose
- Easy to forget in new error paths

### Option 3: tempfile Crate

Use the `tempfile` crate which handles cleanup automatically:

```rust
use tempfile::TempDir;

pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
) -> Result<(Browser, JoinHandle<()>)> {
    // Create temp directory (auto-cleanup on drop)
    let temp_dir = TempDir::new_in(std::env::temp_dir())?;
    let user_data_dir = temp_dir.path().to_path_buf();

    // ... launch browser ...

    // Success - prevent auto-cleanup
    let persistent_path = temp_dir.into_path();

    Ok((browser, handler_task))
}
```

But then we need to track the persistent path for cleanup in BrowserWrapper.

### Option 4: Accept the Leak

Document that failed launches leak temp directories and advise periodic cleanup:

```rust
/// Launch browser instance
///
/// # Temp Directory Cleanup
///
/// Creates a temporary Chrome profile directory. On success, the directory
/// is managed by BrowserWrapper and cleaned up on shutdown. On failure,
/// the directory may be leaked. Periodic cleanup of temp directories
/// starting with "enigo_chrome_" or "kodegen_browser_" is recommended.
```

Add a cleanup script for users:
```bash
# cleanup-browser-temps.sh
rm -rf /tmp/enigo_chrome_*
rm -rf /tmp/kodegen_browser_*
```

## Recommended Solution

**Option 1 (RAII Guard)** is the best approach:
- Automatic and foolproof
- Rust-idiomatic
- No ongoing maintenance burden
- Works for all error paths

## Related Code
- `src/browser/wrapper.rs:52-63` - cleanup_temp_dir (success path)
- `src/manager.rs:192-193` - Calls cleanup_temp_dir after wait
- `src/browser_setup.rs:163-197` - download_managed_browser (also creates temp dirs)

## Testing Recommendations

1. Test that failed launches clean up temp directories
2. Verify temp dir count doesn't grow after failures
3. Test various failure scenarios (missing Chrome, port conflict, etc.)
4. Monitor temp directory usage in production

## Dependencies

Option 3 requires:
```toml
[dependencies]
tempfile = "3.8"
```

## Priority Justification

**Medium** because:
- Causes resource leak
- Accumulates over time
- More severe in high-failure environments (dev/test)
- But doesn't affect correctness
- Relatively easy fix with RAII guard
