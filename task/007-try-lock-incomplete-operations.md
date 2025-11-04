# Incomplete Operations Due to try_lock Usage

## Priority: MEDIUM

## Location
- `src/research/session_manager.rs:163-175` (list_sessions)
- `src/research/session_manager.rs:190-204` (cleanup_old_sessions)

## Issue Description
Both `list_sessions` and `cleanup_old_sessions` use `try_lock()` which silently skips sessions that are currently locked. This can lead to:
1. Incomplete session listings shown to users
2. Sessions never being cleaned up if they're frequently accessed
3. Silent data inconsistency

## Code Reference

### Issue 1: list_sessions
```rust
pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
    let mut sessions = Vec::new();
    for entry in self.sessions.iter() {
        if let Ok(session) = entry.value().try_lock() {  // Line 163 - SKIPS locked sessions!
            sessions.push(serde_json::json!({
                "session_id": session.session_id,
                "query": session.query,
                "status": session.status,
                "started_at": session.started_at.elapsed().as_millis() as u64,
                "runtime_seconds": session.runtime_seconds(),
                "pages_visited": session.progress.last().map(|p| p.pages_visited).unwrap_or(0),
                "current_step": session.progress.last().map(|p| p.message.clone()).unwrap_or_default(),
            }));
        }
        // Silently skips sessions that couldn't be locked!
    }
    sessions
}
```

### Issue 2: cleanup_old_sessions
```rust
async fn cleanup_old_sessions(&self) {
    let mut to_remove = Vec::new();

    for entry in self.sessions.iter() {
        if let Ok(session) = entry.value().try_lock()  // Line 194 - SKIPS locked sessions!
            && session.started_at.elapsed() > SESSION_TIMEOUT
                && session.status != ResearchStatus::Running {
                to_remove.push(session.session_id.clone());
            }
    }

    for session_id in to_remove {
        self.sessions.remove(&session_id);
    }
}
```

## Problem Details

### Issue 1: Incomplete List Results

**User Impact**: When calling `list_research_sessions`, the result is incomplete
- Active sessions being updated won't appear in list
- User sees partial state
- Misleading metrics (appears to have fewer sessions than reality)

**Example Scenario**:
```
1. User has 5 research sessions running
2. User calls list_research_sessions
3. 2 sessions are being updated (locked)
4. User sees only 3 sessions
5. User thinks 2 sessions disappeared
```

### Issue 2: Sessions Never Cleaned Up

**Resource Impact**: Old sessions may never be cleaned up if they're frequently accessed

**Example Scenario**:
```
1. Session completes at T=0
2. Cleanup task runs at T=60s
3. User polls session status at T=59s (acquires lock)
4. Cleanup runs at T=60s (try_lock fails, skips)
5. User polls again at T=119s
6. Cleanup runs at T=120s (try_lock fails again)
7. Session never cleaned up due to frequent polling
```

**Pathological Case**:
- If a session is polled more frequently than every 60 seconds
- And polling takes >1ms
- Cleanup task may never acquire the lock
- Session lives forever (memory leak)

## Impact on Production

### Severity: Medium

**list_sessions**:
- Frequency: Every time a user calls list tool
- Impact: Incorrect information shown to user
- Severity: Medium (confusing but not breaking)

**cleanup_old_sessions**:
- Frequency: Background task every 60 seconds
- Impact: Memory leak if sessions never cleaned up
- Severity: Medium-High (unbounded memory growth over time)

### Real-World Numbers

With default `SESSION_TIMEOUT = 300s` (5 minutes):
- If cleanup consistently fails for a session
- And user creates 1 session/minute
- After 1 hour: 60 uncleaned sessions
- Memory per session: ~10KB (rough estimate)
- Memory leaked: ~600KB/hour (not catastrophic but grows)

In a long-running server:
- 24 hours: ~14MB
- 1 week: ~100MB
- 1 month: ~400MB

## Root Cause Analysis

The `try_lock()` usage appears to be defensive programming to avoid:
1. Deadlocks
2. Long waits if a session is stuck

However, it creates worse problems than it solves.

## Recommended Fixes

### Option 1: Use Async Lock (Recommended)

Wait for locks with a timeout:

```rust
pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
    let mut sessions = Vec::new();
    for entry in self.sessions.iter() {
        // Try to lock with timeout
        let session = match tokio::time::timeout(
            Duration::from_millis(100),
            entry.value().lock()
        ).await {
            Ok(guard) => guard,
            Err(_) => {
                warn!("Session {} timed out during list, skipping", entry.key());
                continue;
            }
        };

        sessions.push(serde_json::json!({
            // ... same fields ...
        }));
    }
    sessions
}
```

**Benefits**:
- Actually waits for lock (completes most of the time)
- Timeout prevents infinite wait
- Logs when sessions are skipped (visibility)

### Option 2: Snapshot Pattern

Store session metadata separately from locked state:

```rust
pub struct ResearchSession {
    // Immutable metadata (no lock needed)
    pub session_id: String,
    pub query: String,
    pub started_at: Instant,

    // Mutable state (locked)
    inner: Arc<Mutex<ResearchSessionInner>>,
}

struct ResearchSessionInner {
    status: ResearchStatus,
    progress: Vec<ResearchStep>,
    results: Vec<ResearchResult>,
    // ...
}
```

Then `list_sessions` can read immutable fields without locking.

### Option 3: Last-Known-Good Cache

Store a cached representation that's updated on each unlock:

```rust
pub struct ResearchSession {
    // ... existing fields ...
    last_snapshot: Arc<RwLock<SessionSnapshot>>,
}

struct SessionSnapshot {
    status: ResearchStatus,
    pages_visited: usize,
    current_step: String,
    updated_at: Instant,
}
```

`list_sessions` reads from cache (RwLock, multiple readers ok).

### Option 4: Accept and Document

If the incomplete behavior is acceptable:

```rust
/// List all active sessions
///
/// Note: Sessions currently being updated may not appear in the list.
/// This is expected behavior to prevent blocking operations.
pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
    // ... existing code ...
}
```

And for cleanup, make it more aggressive:

```rust
async fn cleanup_old_sessions(&self) {
    // ... existing code with try_lock ...

    // Second pass: force cleanup of very old sessions
    let very_old_threshold = SESSION_TIMEOUT * 3; // 15 minutes
    for entry in self.sessions.iter() {
        if let Ok(session) = entry.value().try_lock()
            && session.started_at.elapsed() > very_old_threshold {
                to_remove.push(session.session_id.clone());
            }
    }
}
```

## Comparison to list_sessions Usage

The tool is called from `src/tools/list_research_sessions.rs`:
```rust
let sessions = manager.list_sessions().await;
```

This is exposed to users, so incomplete results are user-visible.

## Related Code
- `src/tools/list_research_sessions.rs` - Tool that calls list_sessions
- `src/tools/get_research_status.rs` - Similar pattern, may have same issue
- `src/research/session_manager.rs:152-157` - stop_session uses async lock (correct)

## Testing Recommendations

1. Test list_sessions while sessions are being updated
2. Verify all sessions appear in list
3. Test cleanup with frequently-polled sessions
4. Verify memory doesn't grow unbounded
5. Load test: create 1000 sessions and verify all get cleaned up

## Priority Justification

**Medium** because:
- Causes visible bugs (incomplete lists)
- Potential memory leak (gradual)
- But not immediately critical
- Workaround exists (wait and list again)
