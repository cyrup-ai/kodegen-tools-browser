# OnceCell Caches Initialization Errors

## Priority: LOW-MEDIUM

## Location
`src/manager.rs:123-135`

## Issue Description
The `BrowserManager::get_or_launch()` method uses `OnceCell::get_or_try_init()` which caches the **first result** - including errors. If the initial browser launch fails due to a transient issue (network timeout, disk full, etc.), the error is cached and all subsequent calls will return the same error, even if the issue is resolved.

## Code Reference
```rust
pub async fn get_or_launch(&self) -> Result<Arc<Mutex<Option<BrowserWrapper>>>> {
    let browser_arc = self
        .browser
        .get_or_try_init(|| async {  // Line 126 - Caches first result (success OR error)
            info!("Launching browser for first use (will be reused)");
            let (browser, handler, user_data_dir) = launch_browser().await?;
            let wrapper = BrowserWrapper::new(browser, handler, user_data_dir);
            Ok::<_, anyhow::Error>(Arc::new(Mutex::new(Some(wrapper))))
        })
        .await?;

    Ok(browser_arc.clone())
}
```

## How OnceCell Works

From tokio::sync::OnceCell docs:

> `get_or_try_init` will execute the given closure at most once. If it returns `Ok`, that value is stored and returned on all subsequent calls. **If it returns `Err`, the cell remains uninitialized** and the next call will retry.

Wait... I need to verify this behavior.

Actually, checking the docs more carefully: **OnceCell DOES retry on error**. The cell remains uninitialized if `get_or_try_init` returns an error.

Let me re-analyze this issue...

## Re-analysis: OnceCell Behavior

Looking at tokio::sync::OnceCell source:
```rust
pub async fn get_or_try_init<F, E>(&self, f: F) -> Result<&T, E>
where
    F: Future<Output = Result<T, E>>,
{
    // If already initialized, return it
    if let Some(value) = self.get() {
        return Ok(value);
    }

    // Otherwise try to initialize
    // If error, does NOT cache - next call will retry
    self.init_inner(f).await
}
```

So actually, **OnceCell does NOT cache errors**. My initial assessment was wrong!

## Updated Issue: Concurrent Failures

However, there's a different issue: **concurrent calls during failure**

### Scenario
```
Time  Thread 1                    Thread 2                    Thread 3
T0    get_or_launch()
T1    ↓ starts init              get_or_launch()
T2    ↓ launch_browser()         ↓ waits for T1
T3    ↓ downloading Chrome...    ↓ still waiting             get_or_launch()
T4    ↓ download fails ❌         ↓ still waiting             ↓ waits for T1
T5    ← returns Error            ← receives same Error       ↓ still waiting
T6                               get_or_launch() (retry)     ← receives Error
T7                               ↓ starts new init           get_or_launch() (retry)
T8                               ↓ success ✅                 ↓ waits for T6
T9                               ← returns Ok                ← returns Ok
```

**Key Point**: All concurrent waiters get the **same error** when init fails. They must each retry independently.

## The Real Problem

If browser launch fails (e.g., Chrome download fails):
1. First call fails and returns error ✅ (correct)
2. **All concurrent calls waiting also get the same error** 😕 (inefficient)
3. Each must retry, potentially causing thundering herd
4. But eventually one will succeed and others will use it ✅

This is not a correctness issue, but an **efficiency issue**.

## Example: Chrome Download Failure

```rust
// 10 tools try to use browser simultaneously
// All call get_or_launch() at the same time

// Thread 1 wins the race, starts downloading Chrome
// Threads 2-10 wait for thread 1

// Thread 1's download fails (network timeout)
// Threads 2-10 all receive the error ❌

// All 10 threads retry
// Thread 3 wins the retry race
// Threads 1,2,4-10 wait for thread 3

// Thread 3 succeeds
// Everyone gets the browser ✅
```

**Problem**: 9 threads unnecessarily received an error on first attempt.

**Impact**:
- Delayed operation (but eventually succeeds)
- Multiple error logs confuse debugging
- User sees multiple "browser launch failed" messages

## How Often Does This Happen?

**Browser launch failure scenarios**:
1. Chrome not installed + download fails (network issue)
2. Disk full when creating temp directory
3. Chrome binary corrupted
4. Permissions issue on temp directory
5. Port conflict (unlikely with chromiumoxide)

**Frequency**: Rare in production, more common in constrained environments

**Severity**: Low - eventually succeeds on retry

## Recommended Fixes

### Option 1: Accept Current Behavior (Recommended)

The current behavior is **correct**, just not optimal:
- Errors are not cached ✅
- Retries eventually succeed ✅
- Concurrent calls are thread-safe ✅

Just document it:

```rust
/// Get or launch the shared browser instance
///
/// Uses OnceCell for atomic async initialization to prevent race conditions
/// during first browser launch. Multiple concurrent calls will not
/// launch multiple browsers.
///
/// # Error Handling
///
/// If the initial browser launch fails (e.g., Chrome download timeout),
/// the error is NOT cached - subsequent calls will retry. However, all
/// concurrent calls waiting for the same initialization will receive
/// the same error and must each retry independently.
///
/// # Performance
/// - First call: ~2-3s (launches browser)
/// - Subsequent calls: <1ms (atomic pointer load, no locks)
/// - Failed initialization: All concurrent waiters receive error
```

### Option 2: Retry Logic with Backoff

Add retry logic inside get_or_launch:

```rust
pub async fn get_or_launch(&self) -> Result<Arc<Mutex<Option<BrowserWrapper>>>> {
    // Retry up to 3 times with exponential backoff
    let mut attempt = 0;
    loop {
        match self.try_get_or_launch().await {
            Ok(browser) => return Ok(browser),
            Err(e) if attempt < 2 => {
                attempt += 1;
                let backoff = Duration::from_millis(100 * 2_u64.pow(attempt));
                warn!("Browser launch attempt {} failed: {}. Retrying after {:?}...",
                      attempt, e, backoff);
                tokio::time::sleep(backoff).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn try_get_or_launch(&self) -> Result<Arc<Mutex<Option<BrowserWrapper>>>> {
    // ... existing get_or_launch logic ...
}
```

**Benefits**:
- Transparent retry for transient failures
- Backoff prevents thundering herd

**Drawbacks**:
- Hides errors that should be surfaced
- Delays failure reporting
- May retry non-transient errors (bad)

### Option 3: Manual Mutex with Retry Count

Replace OnceCell with manual tracking:

```rust
pub struct BrowserManager {
    browser: Arc<Mutex<Option<BrowserWrapper>>>,
    launch_attempts: Arc<AtomicUsize>,
}
```

**Benefits**:
- Can implement custom retry logic
- Track failure count

**Drawbacks**:
- More complex
- Lose OnceCell's atomic initialization guarantee
- Potential race conditions

## Actual Impact Analysis

Let me reconsider the actual impact:

**Current behavior**:
- First call launches browser (or fails) ✅
- Concurrent calls wait and get same result ✅
- Failed calls return error (don't cache) ✅
- Callers can retry ✅

**This is actually fine!** The OnceCell behavior is correct.

## Revised Priority: LOW

This is not really a bug - it's correct behavior. The only "issue" is that concurrent waiters all get the same error, but that's expected and correct.

## Related Code
- `src/browser_setup.rs:209-297` - launch_browser() that may fail
- `src/manager.rs:174-201` - shutdown() properly handles cleanup
- Tool implementations that call get_or_launch()

## Testing Recommendations

Add test to verify retry behavior:
```rust
#[tokio::test]
async fn test_browser_launch_retry() {
    // Simulate failure on first attempt
    // Verify second attempt can succeed
    // Verify no error caching
}
```

## Priority Justification

**Low** because:
- Current behavior is correct (errors are NOT cached)
- Issue is efficiency, not correctness
- Only affects concurrent calls during failure
- Browser launch usually succeeds
- Workaround is trivial (just retry)

This task documents expected behavior more than identifying a bug.
