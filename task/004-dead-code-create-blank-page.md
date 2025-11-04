# Dead Code: create_blank_page Function

## Priority: LOW

## Location
`src/browser/wrapper.rs:132-142`

## Issue Description
The `create_blank_page` function is marked with `#[allow(dead_code)]`, indicating it's not used anywhere in the codebase. This adds maintenance burden without providing value.

## Code Reference
```rust
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
#[allow(dead_code)]
pub async fn create_blank_page(wrapper: &BrowserWrapper) -> Result<Page> {
    let page = wrapper
        .browser()
        .new_page("about:blank")
        .await
        .context("Failed to create blank page")?;

    info!("Created blank page for stealth injection");
    Ok(page)
}
```

## Evidence of Dead Code

1. **Compiler Attribute**: `#[allow(dead_code)]` explicitly marks it as unused
2. **Documentation Pattern**: Comment says "Based on packages/citescrape/..." suggesting it was copied but not integrated
3. **Purpose Mismatch**: The function is for "stealth injection" but kromekover handles that differently

## Code Purpose
The function appears to be for a pattern where:
1. Create blank page
2. Apply stealth measures
3. Navigate to target URL

However, the current codebase uses a different approach where stealth is applied automatically via kromekover during browser launch, not per-page.

## Impact on Production

**Code Health**:
- **Maintenance**: Dead code must be read and understood by developers
- **Confusion**: Suggests a usage pattern that doesn't exist
- **False Documentation**: Comments describe a workflow that isn't used

**Runtime**: No impact - function is never called

## Analysis

The function is only 11 lines and simple, but:
1. It's exported as public, suggesting it's part of the API
2. The comment references "citescrape" pattern suggesting it was ported but not needed
3. The about:blank pattern may have been needed in citescrape but not here

## Current Page Creation Pattern

The actual pattern used in the codebase:
```rust
// From tools/navigate.rs (typical usage)
let page = browser.new_page(&args.url).await?;
// Kromekover already applied via browser launch
```

Pages are created directly with target URLs, not through blank pages.

## Recommended Action

**Option 1: Remove entirely** (recommended)
- Delete lines 118-142
- Remove from public API
- No impact on functionality

**Option 2: Make it useful**
- Actually use it somewhere if the pattern is valuable
- But current kromekover approach seems sufficient

**Option 3: Document why it exists**
- If kept for future use, explain why
- Add TODO or FIXME comment

## Related Code
- `src/kromekover/mod.rs` - Current stealth injection approach
- `src/tools/navigate.rs` - How pages are actually created
- `src/browser/wrapper.rs:160-171` - `get_current_page` (similar but actually used)

## Recommendation
**Remove** - The function serves no purpose and the about:blank pattern isn't needed with the current kromekover approach that applies stealth at browser launch time.
