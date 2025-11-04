# Session Cancel Doesn't Wait for Task Completion

## Priority: LOW-MEDIUM

## Location
`src/research/session_manager.rs:105-111`

## Issue Description
The `ResearchSession::cancel()` method aborts the background task but doesn't wait for it to complete. This can leave resources in an inconsistent state and doesn't give the task a chance to clean up gracefully.

## Code Reference
```rust
/// Mark as cancelled
pub fn cancel(&mut self) {
    self.status = ResearchStatus::Cancelled;
    if let Some(handle) = self.task_handle.take() {
        handle.abort();  // Line 109 - Aborts immediately, doesn't wait
    }
}
```

## Problem Details

### Issue 1: No Cleanup Opportunity
When `abort()` is called:
1. Task is terminated immediately at the next `.await` point
2. No destructors run for non-RAII resources
3. No graceful shutdown opportunity

### Issue 2: Resource Leak Risk
The research task may have:
- Browser pages open
- Network connections active
- Temporary data in memory
- Locks held on shared resources

### Issue 3: Inconsistent State
```rust
// Research task in progress:
async fn research_task() {
    let page = browser.new_page(url).await?;  // Page created
    let results = extract_data(page).await?;  // Aborted here!
    save_results(results).await?;             // Never reached
    page.close().await?;                       // Never reached - page leaked!
}
```

## Current Usage

The cancel method is called from:
```rust
// src/research/session_manager.rs:152-157
pub async fn stop_session(&self, session_id: &str) -> Result<()> {
    let session_ref = self.get_session(session_id).await?;
    let mut session = session_ref.lock().await;
    session.cancel();  // Returns immediately, doesn't wait
    Ok(())
}
```

And exposed to users via `src/tools/stop_browser_research.rs`.

## Impact on Production

### Severity: Low-Medium

**Resource Impact**:
- Browser pages may not be closed (but BrowserManager cleanup will eventually handle it)
- Memory may not be freed immediately
- Locks may be held briefly (but will be released when task exits)

**User Impact**:
- User calls stop_research
- Gets immediate success response
- But research may still be running for a few milliseconds
- Confusing if they immediately check status

### Example Scenario
```
User: Stop research session ABC
System: ✓ Session stopped

User: List sessions
System: Session ABC - Status: Running  ← Still running!

[100ms later]
User: List sessions
System: Session ABC - Status: Cancelled  ← Now stopped
```

## Comparison to Agent Implementation

The agent code (`src/agent/core.rs:206-260`) does this **correctly**:

```rust
pub async fn stop(&self) -> AgentResult<()> {
    // Send stop command
    self.command_channel.send(AgentCommand::Stop).await?;

    // Wait for Stopped confirmation with timeout
    let mut receiver = self.response_channel.lock().await;
    match tokio::time::timeout(Duration::from_secs(5), receiver.recv()).await {
        Ok(Some(AgentResponse::Stopped)) => Ok(()),
        // ... handle other cases ...
    }
}
```

The agent:
1. Sends a stop command (graceful)
2. Waits for confirmation (synchronous)
3. Has a timeout (safety)

## Recommended Fixes

### Option 1: Graceful Cancellation with Timeout (Recommended)

Make cancel async and wait for task completion:

```rust
/// Cancel the session and wait for task to stop
///
/// Attempts graceful cancellation by aborting the task and waiting for it
/// to complete. If the task doesn't complete within 5 seconds, logs a warning
/// but continues anyway.
pub async fn cancel(&mut self) -> Result<()> {
    self.status = ResearchStatus::Cancelled;

    if let Some(handle) = self.task_handle.take() {
        // Abort the task
        handle.abort();

        // Wait for it to complete (with timeout)
        match tokio::time::timeout(Duration::from_secs(5), handle).await {
            Ok(Ok(())) => {
                info!("Research task cancelled gracefully");
            }
            Ok(Err(e)) if e.is_cancelled() => {
                // Expected - task was aborted
                info!("Research task cancelled via abort");
            }
            Ok(Err(e)) => {
                warn!("Research task exited with error during cancel: {}", e);
            }
            Err(_) => {
                warn!("Research task did not complete within 5s of abort");
                // Continue anyway - task will be dropped
            }
        }
    }

    Ok(())
}
```

Update call site:
```rust
pub async fn stop_session(&self, session_id: &str) -> Result<()> {
    let session_ref = self.get_session(session_id).await?;
    let mut session = session_ref.lock().await;
    session.cancel().await?;  // Now waits
    Ok(())
}
```

**Benefits**:
- Waits for task to actually stop
- User gets confirmation when really stopped
- Resources cleaned up before returning

**Trade-offs**:
- Takes up to 5 seconds in worst case
- But that's acceptable for a stop operation

### Option 2: Cancellation Token (Better Graceful Shutdown)

Use a cancellation token that the task checks:

```rust
pub struct ResearchSession {
    // ... existing fields ...
    cancel_token: CancellationToken,
    task_handle: Option<JoinHandle<()>>,
}

impl ResearchSession {
    pub fn new(session_id: String, query: String) -> Self {
        Self {
            // ... existing init ...
            cancel_token: CancellationToken::new(),
            task_handle: None,
        }
    }

    /// Cancel gracefully
    pub async fn cancel(&mut self) -> Result<()> {
        self.status = ResearchStatus::Cancelled;

        // Signal cancellation
        self.cancel_token.cancel();

        // Wait for task to acknowledge and exit
        if let Some(handle) = self.task_handle.take() {
            match tokio::time::timeout(Duration::from_secs(10), handle).await {
                Ok(_) => info!("Research task stopped gracefully"),
                Err(_) => {
                    warn!("Research task didn't stop in time, may still be running");
                }
            }
        }

        Ok(())
    }
}

// In the research task:
async fn do_research(cancel_token: CancellationToken, ...) {
    loop {
        // Check cancellation regularly
        if cancel_token.is_cancelled() {
            info!("Research cancelled, cleaning up...");
            cleanup().await;
            break;
        }

        // Do research work...
        process_next_url().await?;
    }
}
```

**Benefits**:
- Task can clean up gracefully
- No resource leaks
- Cooperative cancellation is safer

**Trade-offs**:
- More complex implementation
- Requires propagating token through research code
- Depends on task checking token regularly

### Option 3: Keep Current Behavior, Document It

If the resource leak risk is acceptable:

```rust
/// Mark as cancelled and abort background task
///
/// WARNING: This uses `abort()` which terminates the task immediately
/// at the next `.await` point. Resources held by the task may not be
/// cleaned up immediately. The task handle is awaited by the tokio
/// runtime, but destructors for non-RAII resources may not run.
///
/// For most use cases this is acceptable because:
/// - Browser pages will be cleaned up by BrowserManager
/// - Locks will be released when task exits
/// - Memory will be freed by tokio runtime
pub fn cancel(&mut self) {
    self.status = ResearchStatus::Cancelled;
    if let Some(handle) = self.task_handle.take() {
        handle.abort();
    }
}
```

## Related Code
- `src/agent/core.rs:206-260` - Agent stop() implementation (correct pattern)
- `src/tools/stop_browser_research.rs` - Tool that calls cancel()
- `src/utils/deep_research.rs` - Research implementation that may need cleanup

## Testing Recommendations

1. Test that cancel() actually stops the task
2. Verify status changes to Cancelled before task stops
3. Test that resources (pages, etc.) are cleaned up
4. Test cancel during different phases of research

## Priority Justification

**Low-Medium** because:
- Resource leak risk is low (browser cleanup handles most)
- But user-visible inconsistency (reports stopped but still running briefly)
- Easy fix (make method async and wait)
- Affects user-facing tool
