# Resource Leak: Untracked Cleanup Task in ResearchSessionManager

## Priority: MEDIUM

## Location
`src/research/session_manager.rs:178-187`

## Issue Description
The `spawn_cleanup_task` method spawns a background task that runs forever but discards the `JoinHandle`. This means there's no way to gracefully stop the task when the application shuts down, leading to a resource leak and potential issues during shutdown.

## Code Reference
```rust
/// Spawn background cleanup task
fn spawn_cleanup_task(&self) {
    tokio::spawn(async {  // Line 180 - JoinHandle is discarded!
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            Self::global().cleanup_old_sessions().await;
        }
    });
}
```

This is called from the singleton initialization at line 128:
```rust
INSTANCE.get_or_init(|| {
    let manager = Self {
        sessions: DashMap::new(),
    };
    // Spawn cleanup task
    manager.spawn_cleanup_task();  // No way to stop it later!
    manager
})
```

## Problem Details

1. **No Handle Stored**: The `JoinHandle` returned by `tokio::spawn` is immediately dropped
2. **No Cancellation**: The task runs `loop { }` forever with no way to break out
3. **Singleton Lifetime**: The task is spawned once and runs for the entire process lifetime
4. **Shutdown Issues**: During process shutdown, the task may still be running

## Impact on Production

### Runtime Behavior
- **Normal Operation**: Task runs every 60 seconds, cleans up old sessions
- **Memory**: Minimal overhead (one tokio task)
- **CPU**: Negligible (runs once per minute)

### Shutdown Behavior
- **Graceful Shutdown**: Task may be in the middle of cleanup
- **Tokio Runtime**: Has to forcefully terminate the task
- **Potential Warnings**: May see "task leaked" or similar warnings

### Real-World Impact
**Severity**: Medium
- Not a memory leak during normal operation
- But prevents clean shutdown
- Could cause issues in test environments
- May delay process termination

## Examples of Issues

### Scenario 1: Test Suite
```rust
#[tokio::test]
async fn test_research() {
    // Creates ResearchSessionManager::global()
    // Spawns cleanup task
    // Test finishes
    // Cleanup task still running!
}
// Next test may see leftover state
```

### Scenario 2: Server Shutdown
```
1. SIGTERM received
2. Server starts graceful shutdown
3. Cleanup task still running in background
4. Tokio runtime waits for task (or times out)
5. Forced termination or delayed shutdown
```

## Recommended Fixes

### Option 1: Store JoinHandle and Abort on Drop (Recommended)

```rust
pub struct ResearchSessionManager {
    sessions: DashMap<String, Arc<tokio::sync::Mutex<ResearchSession>>>,
    cleanup_task: Option<JoinHandle<()>>,  // NEW
}

impl ResearchSessionManager {
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<ResearchSessionManager> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            let cleanup_handle = Self::spawn_cleanup_task();
            Self {
                sessions: DashMap::new(),
                cleanup_task: Some(cleanup_handle),
            }
        })
    }

    fn spawn_cleanup_task() -> JoinHandle<()> {
        tokio::spawn(async {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                Self::global().cleanup_old_sessions().await;
            }
        })
    }
}

impl Drop for ResearchSessionManager {
    fn drop(&mut self) {
        if let Some(handle) = self.cleanup_task.take() {
            handle.abort();
        }
    }
}
```

**Issue**: Singleton never drops (lives until process end), so Drop won't be called.

### Option 2: Cancellation Token (Better)

```rust
use tokio_util::sync::CancellationToken;

pub struct ResearchSessionManager {
    sessions: DashMap<String, Arc<tokio::sync::Mutex<ResearchSession>>>,
    cleanup_token: CancellationToken,
    cleanup_task: JoinHandle<()>,
}

impl ResearchSessionManager {
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<ResearchSessionManager> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            let token = CancellationToken::new();
            let cleanup_handle = Self::spawn_cleanup_task(token.clone());
            Self {
                sessions: DashMap::new(),
                cleanup_token: token,
                cleanup_task: cleanup_handle,
            }
        })
    }

    fn spawn_cleanup_task(cancel_token: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        Self::global().cleanup_old_sessions().await;
                    }
                    _ = cancel_token.cancelled() => {
                        info!("Cleanup task cancelled");
                        break;
                    }
                }
            }
        })
    }

    /// Shutdown cleanup task gracefully
    pub async fn shutdown(&self) -> Result<()> {
        self.cleanup_token.cancel();
        // Wait for task with timeout
        if let Err(e) = tokio::time::timeout(
            Duration::from_secs(5),
            &self.cleanup_task
        ).await {
            warn!("Cleanup task didn't stop within timeout: {}", e);
        }
        Ok(())
    }
}
```

### Option 3: Accept the Limitation (Not Recommended)

Document that the task runs for process lifetime:
```rust
/// Spawn background cleanup task
///
/// NOTE: This task runs for the entire process lifetime and cannot be stopped.
/// This is acceptable because it's lightweight (runs once per minute) and
/// the process will terminate it on exit.
fn spawn_cleanup_task(&self) {
    // ...
}
```

## Comparison to BrowserManager

`BrowserManager` has a similar pattern but handles it better:
- Has explicit `shutdown()` method (line 174)
- Implements `ShutdownHook` trait (line 229)
- Server can call shutdown during graceful termination

`ResearchSessionManager` should follow the same pattern.

## Related Code
- `src/manager.rs:174-235` - BrowserManager shutdown pattern (good example)
- `src/research/session_manager.rs:105-111` - Session cancel() also has task abort
- `src/agent/core.rs:206-260` - Agent has proper stop() implementation

## Testing Recommendations

1. Add test that verifies cleanup task can be stopped
2. Test that shutdown completes within reasonable time
3. Check for task leaks in test suite
4. Verify no warnings during process exit

## Dependencies

If using Option 2:
```toml
[dependencies]
tokio-util = { version = "0.7", features = ["sync"] }
```
