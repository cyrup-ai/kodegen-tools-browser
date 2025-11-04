# Dead Code: apply_stealth_measures Function

## Priority: LOW

## Location
`src/browser_setup.rs:299-437`

## Issue Description
The `apply_stealth_measures` function is exported as public but never called internally or externally. This is explicitly documented in the code comment at line 304-306, indicating it's legacy code that has been superseded by the kromekover system.

## Code Reference
```rust
/// Apply stealth mode settings to evade bot detection
///
/// Injects JavaScript to modify navigator properties, WebGL fingerprinting,
/// and Chrome-specific APIs. All operations are instant via CDP.
///
/// Note: This function is exported for external use but not called internally.
/// The production stealth implementation is `kromekover::inject()` which offers
/// more comprehensive evasions.
pub async fn apply_stealth_measures(page: &chromiumoxide::Page) -> Result<()> {
    // ... 138 lines of JavaScript injection code ...
}
```

## Evidence of Dead Code

1. **Explicit Comment**: Lines 304-306 state it's "not called internally" and that kromekover is the "production stealth implementation"
2. **Grep Search**: Only found in `src/browser_setup.rs`, no call sites
3. **Superseded**: kromekover system (lines 12-20 in various files) provides more comprehensive stealth

## Code Size
- **Lines**: 138 lines (299-437)
- **Functionality**: JavaScript injection for:
  - navigator.webdriver removal
  - User agent override
  - Languages spoofing
  - Plugins mocking
  - chrome.runtime mocking
  - WebGL vendor spoofing

## Impact on Production

**Code Health**:
- **Maintenance burden**: Dead code must be understood and maintained
- **Confusion**: Developers may wonder which stealth system to use
- **Binary size**: Adds ~10KB of JavaScript strings to binary
- **API surface**: Public function suggests it's supported

**Runtime**: No impact - function is never called

## Comparison with kromekover

The kromekover system (`src/kromekover/mod.rs`) provides:
- 20+ evasion scripts (vs 6 in apply_stealth_measures)
- Coordinated injection order
- More comprehensive coverage
- Active maintenance

**Overlap**: apply_stealth_measures covers a subset of what kromekover does:
- `navigator.webdriver` → `kromekover/evasions/navigator_webdriver.js`
- `navigator.languages` → `kromekover/evasions/navigator_language.js`
- `navigator.plugins` → `kromekover/evasions/navigator_plugins.js`
- `chrome.runtime` → `kromekover/evasions/chrome_runtime.js`
- WebGL → `kromekover/evasions/webgl_vendor_override.js`

## Recommended Action

**Option 1: Remove entirely** (recommended)
- Delete lines 299-437
- Remove from public API
- Simplifies codebase
- Forces users to kromekover system

**Option 2: Deprecate**
- Add `#[deprecated]` attribute
- Update docs to point to kromekover
- Remove in next major version

**Option 3: Keep as example**
- Move to `examples/` directory
- Keep as reference implementation
- Remove from main library

## Migration Path

If any external users depend on this function:
1. Update them to use kromekover injection instead
2. kromekover is already active on all browser launches (via `launch_browser`)

## Related Code
- `src/kromekover/mod.rs` - Current production stealth system
- `src/kromekover/evasions/*.js` - Individual evasion scripts
- `src/page_enhancer.rs` - Wrapper that applies kromekover

## Business Impact
- **Risk**: None - function is unused
- **Benefit**: Cleaner codebase, reduced maintenance
- **Breaking Change**: Only if external code calls this function (unlikely based on comment)
