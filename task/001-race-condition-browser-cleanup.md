# Race Condition in BrowserWrapper Cleanup

## Priority: HIGH

## Location
`src/browser/wrapper.rs:74-88`

## Issue Description
The `Drop` implementation for `BrowserWrapper` has a race condition where it attempts to cleanup the temp directory immediately after aborting the handler task, without waiting for the browser process to fully terminate. This can cause cleanup failures on Windows where file handles remain locked.

## Code Reference
```rust
impl Drop for BrowserWrapper {
    fn drop(&mut self) {
        info!("Dropping BrowserWrapper - aborting handler task");
        self.handler.abort();  // Line 77
        // Handler will be awaited/cleaned up by tokio runtime
        // Browser::drop() will automatically kill the Chrome process

        // Cleanup temp directory (fallback if shutdown() wasn't called)
        if self.user_data_dir.is_some() {
            tracing::warn!(
                "BrowserWrapper dropped without explicit cleanup - removing temp dir in Drop"
            );
            self.cleanup_temp_dir();  // Line 86 - RACE: Chrome may not have released file handles
        }
    }
}
```

## Problem Details

1. **No Synchronization**: `handler.abort()` only signals the task to stop but doesn't wait for it to complete
2. **File Handle Lock**: Chrome process may still hold locks on profile files when cleanup runs
3. **Platform-Specific**: Particularly problematic on Windows which fails to remove locked files
4. **Comment Contradiction**: The code comments at lines 43-49 explicitly state "MUST be called AFTER `browser.wait()` completes", but the Drop implementation doesn't follow this

## Comment from the Code
```rust
/// Clean up temp directory (blocking operation)
///
/// MUST be called AFTER `browser.wait()` completes to ensure Chrome
/// has released all file handles. Windows will fail to remove locked files.
```

## Impact on Production

- **Runtime**: Cleanup failures logged as warnings but temp directories accumulate
- **Resource Leak**: Multiple failed cleanups can fill up temp directory with orphaned Chrome profiles
- **Windows Users**: More likely to see cleanup failures
- **Server Environments**: Long-running processes accumulate temp directories

## Recommended Fix

The Drop implementation should be a best-effort cleanup only, with a clear warning that it may fail. The proper cleanup path through `BrowserManager::shutdown()` already handles this correctly (lines 187-193 in manager.rs):

```rust
// 2. Wait for process to fully exit (CRITICAL - releases file handles)
if let Err(e) = wrapper.browser_mut().wait().await {
    tracing::warn!("Failed to wait for browser exit: {}", e);
}

// 3. Cleanup temp directory
wrapper.cleanup_temp_dir();
```

**Option 1: Remove cleanup from Drop** (cleanest)
- Remove lines 82-87 from Drop implementation
- Force users to call explicit shutdown
- Document that Drop doesn't cleanup temp dir

**Option 2: Make Drop cleanup best-effort** (pragmatic)
- Keep the warning
- Document that cleanup may fail
- Add more explicit logging about why it might fail

**Option 3: Block on runtime** (risky)
- Try to get a runtime handle and block on browser.wait()
- This is complex and can cause panics if no runtime exists

## Related Code
- `src/manager.rs:174-201` - Correct shutdown implementation
- `src/browser/wrapper.rs:52-63` - cleanup_temp_dir method with comments

## Testing Recommendations
1. Create test that drops BrowserWrapper without calling shutdown
2. Verify temp directory cleanup on Windows
3. Check for orphaned Chrome processes
4. Monitor temp directory growth in long-running servers
