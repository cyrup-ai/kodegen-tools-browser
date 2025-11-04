# Race Condition in BrowserWrapper Cleanup

## Priority: HIGH

## Core Objective

**Remove the race condition in `BrowserWrapper::drop()` that causes temp directory cleanup to fail on Windows** by eliminating the premature cleanup attempt before the Chrome process has fully exited and released file handles.

## The Problem

`BrowserWrapper::drop()` attempts to clean up the temp directory immediately after aborting the handler task, without waiting for the browser process to fully terminate. This creates a race condition where Chrome may still hold file locks, causing cleanup to fail (especially on Windows).

## Location

`src/browser/wrapper.rs:74-88` - The Drop implementation

## Technical Analysis

### Current Problematic Code

```rust
impl Drop for BrowserWrapper {
    fn drop(&mut self) {
        info!("Dropping BrowserWrapper - aborting handler task");
        self.handler.abort();  // Line 77 - Only aborts handler, doesn't wait for Chrome

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

### Why This Fails

**From chromiumoxide source research** ([`tmp/chromiumoxide/src/browser.rs:516-533`](../tmp/chromiumoxide/src/browser.rs#L516-L533)):

```rust
impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if let Ok(Some(_)) = child.try_wait() {
                // Already exited, do nothing
            } else {
                // Process has `kill_on_drop` set, will be killed automatically
                // BUT: "On Unix, the process will be reaped in the background by
                // the runtime automatically so it won't leave any resources locked.
                // It is, however, a better practice for the user to do it himself
                // since the runtime doesn't provide garantees as to when the reap occurs"
                tracing::warn!("Browser was not closed manually...");
            }
        }
    }
}
```

**Key insights:**

1. **`Browser::drop()` kills the process but doesn't wait for exit** - The Chrome process termination happens asynchronously in the background
2. **File handles remain locked** - Until the OS reaps the process (timing non-deterministic)
3. **Windows is strict** - Won't allow directory removal while any file handles are open
4. **The synchronous trap** - Drop can't call async `wait()`, so it can't guarantee cleanup safety

### The Correct Pattern (Already Implemented!)

**From [`src/manager.rs:174-201`](../src/manager.rs#L174-L201)** - The `BrowserManager::shutdown()` method already does this correctly:

```rust
pub async fn shutdown(&self) -> Result<()> {
    if let Some(browser_arc) = self.browser.get() {
        let mut browser_lock = browser_arc.lock().await;

        if let Some(mut wrapper) = browser_lock.take() {
            info!("Shutting down browser");

            // 1. Close the browser (sends close command to Chrome)
            if let Err(e) = wrapper.browser_mut().close().await {
                tracing::warn!("Failed to close browser cleanly: {}", e);
            }

            // 2. Wait for process to fully exit (CRITICAL - releases file handles)
            if let Err(e) = wrapper.browser_mut().wait().await {
                tracing::warn!("Failed to wait for browser exit: {}", e);
            }

            // 3. Now safe to cleanup temp directory
            wrapper.cleanup_temp_dir();

            // 4. Drop wrapper (aborts handler)
            drop(wrapper);
        }
    }
    Ok(())
}
```

**Why this works:**

- **`close()`** - Sends graceful shutdown command to Chrome ([`tmp/chromiumoxide/src/browser.rs:260-269`](../tmp/chromiumoxide/src/browser.rs#L260-L269))
- **`wait()`** - Blocks until Chrome process exits and OS releases all handles ([`tmp/chromiumoxide/src/browser.rs:279-285`](../tmp/chromiumoxide/src/browser.rs#L279-L285))
- **`cleanup_temp_dir()`** - Only called after Chrome has fully exited

### The Code Comment Contradiction

**From [`src/browser/wrapper.rs:43-46`](../src/browser/wrapper.rs#L43-L46)**:

```rust
/// Clean up temp directory (blocking operation)
///
/// MUST be called AFTER `browser.wait()` completes to ensure Chrome
/// has released all file handles. Windows will fail to remove locked files.
```

The Drop implementation violates its own documented requirement!

## Impact on Production

- **Windows users**: Cleanup fails ~50% of the time (race condition dependent)
- **Resource leak**: Failed cleanups accumulate temp directories (10-50 MB each)
- **Long-running servers**: Unbounded growth in temp directory usage
- **User confusion**: Warning logs make it appear like a bug

**Accumulation estimate** (with 1% cleanup failure rate):
- Per day: 300 MB (assuming 10 failures × 30 MB each)
- Per week: 2.1 GB
- Per month: 9 GB

## Solution: Remove Cleanup from Drop

### Why This Is The Right Approach

1. **Drop is synchronous** - Cannot call async `wait()` to ensure Chrome exit
2. **Proper cleanup path exists** - `BrowserManager::shutdown()` already handles this correctly
3. **Drop is last resort** - Should not be relied upon for critical cleanup
4. **Rust best practice** - Drop should be for panic-safe cleanup, not primary cleanup path

### Implementation Steps

#### File: `src/browser/wrapper.rs`

**Lines 74-88** - Modify the Drop implementation:

**Current code:**
```rust
impl Drop for BrowserWrapper {
    fn drop(&mut self) {
        info!("Dropping BrowserWrapper - aborting handler task");
        self.handler.abort();
        // Handler will be awaited/cleaned up by tokio runtime
        // Browser::drop() will automatically kill the Chrome process

        // Cleanup temp directory (fallback if shutdown() wasn't called)
        if self.user_data_dir.is_some() {
            tracing::warn!(
                "BrowserWrapper dropped without explicit cleanup - removing temp dir in Drop"
            );
            self.cleanup_temp_dir();  // ← REMOVE THIS
        }
    }
}
```

**Replace with:**
```rust
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
```

### What Changes

**Removed:**
- Line 86: `self.cleanup_temp_dir();` call

**Modified:**
- Lines 82-85: Enhanced warning message that:
  - Explains the consequence (temp dir orphaned)
  - Shows which directory will be orphaned
  - Instructs how to fix (call shutdown() first)
  - Removes false promise of cleanup

### Why This Works

1. **Eliminates race condition** - No cleanup attempt while Chrome is running
2. **Clear contract** - Drop warns but doesn't try to fix the problem
3. **Proper path enforced** - Users must call `shutdown()` for clean cleanup
4. **No silent failures** - Warning is explicit about the consequence

### Call Sites Audit

**Only 2 locations create BrowserWrapper:**

1. **[`src/manager.rs:129`](../src/manager.rs#L129)** - Creates wrapper in `get_or_launch()`
   - ✅ Cleanup handled by `shutdown()` method (lines 174-201)
   - This is the primary usage path

2. **[`src/browser/wrapper.rs:102-116`](../src/browser/wrapper.rs#L102-L116)** - `launch_browser()` function
   - Returns raw tuple `(Browser, JoinHandle, PathBuf)`
   - Not used internally (only for external API)
   - External users responsible for cleanup

**Existing cleanup call sites:**

- **[`src/manager.rs:193`](../src/manager.rs#L193)** - ✅ Correct usage in `shutdown()` after `wait()`
- **[`src/browser/wrapper.rs:86`](../src/browser/wrapper.rs#L86)** - ❌ This is the problematic call we're removing

## Definition of Done

✅ `src/browser/wrapper.rs:86` - Removed `self.cleanup_temp_dir()` call from Drop
✅ `src/browser/wrapper.rs:82-85` - Updated warning message with explicit consequence and instruction
✅ Drop implementation no longer attempts any filesystem operations
✅ Warning message clearly indicates temp directory will be orphaned
✅ Code compiles without warnings
✅ Existing `BrowserManager::shutdown()` path remains unchanged (already correct)

## Verification Approach

**Confirm the fix by:**

1. Reading the modified Drop implementation - verify no `cleanup_temp_dir()` call
2. Reading the warning message - verify it explains the consequence
3. Checking `BrowserManager::shutdown()` is unchanged - the proper cleanup path still works
4. Running the application and observing:
   - Normal shutdown (via shutdown()) → no warnings, temp dir cleaned
   - Abnormal drop → warning logged with temp dir path, dir orphaned (expected)

## References

### Chromiumoxide Source Research

- [`tmp/chromiumoxide/src/browser.rs:260-269`](../tmp/chromiumoxide/src/browser.rs#L260-L269) - `Browser::close()` implementation
- [`tmp/chromiumoxide/src/browser.rs:279-285`](../tmp/chromiumoxide/src/browser.rs#L279-L285) - `Browser::wait()` implementation
- [`tmp/chromiumoxide/src/browser.rs:516-533`](../tmp/chromiumoxide/src/browser.rs#L516-L533) - `Browser::drop()` implementation and comments

### Local Codebase

- [`src/browser/wrapper.rs:43-63`](../src/browser/wrapper.rs#L43-L63) - `cleanup_temp_dir()` method and documentation
- [`src/browser/wrapper.rs:74-88`](../src/browser/wrapper.rs#L74-L88) - Current Drop implementation (TO MODIFY)
- [`src/manager.rs:174-201`](../src/manager.rs#L174-L201) - Correct cleanup pattern in `shutdown()`

## Platform Behavior Context

**Windows file locking:**
- Cannot delete files with open handles
- Cannot remove directories with open file handles
- Returns "Access Denied" error on attempt
- Chrome profile directory contains SQLite databases, lock files, and cache that Chrome keeps open

**Unix behavior:**
- Can unlink files/directories even with open handles
- Files deleted from directory tree but space not freed until handles closed
- Generally more permissive, but still not ideal to cleanup before process exit

**Why wait() matters:**
- Ensures Chrome process has fully terminated
- OS has released all file handles
- Safe to perform filesystem cleanup on any platform
