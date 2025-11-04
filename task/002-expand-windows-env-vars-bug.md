# Bug in expand_windows_env_vars: Malformed Input Handling

## Priority: MEDIUM

## Core Objective

**Fix the `expand_windows_env_vars` function to properly detect when a closing `%` is missing**, preventing silent string corruption and incorrect path reconstruction when malformed environment variable tokens are encountered.

## The Problem

The function uses `.take_while(|&c| c != '%')` which consumes characters until finding a `%` or reaching end-of-string, but **never checks whether a closing `%` was actually found**. This causes three issues:

1. **Silent consumption**: If no closing `%` exists, the entire rest of the string becomes the "variable name"
2. **Incorrect reconstruction**: Always adds a closing `%` even when it wasn't in the input
3. **Path corruption**: Malformed paths get silently mangled instead of being preserved

## Location

**File**: [`src/browser_setup.rs:130-159`](../src/browser_setup.rs#L130-L159)

## Current Implementation

```rust
fn expand_windows_env_vars(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            // Found start of potential environment variable
            let var_name: String = chars.by_ref().take_while(|&c| c != '%').collect();  // ← BUG

            if !var_name.is_empty() {
                // Try to expand the variable
                if let Ok(value) = std::env::var(&var_name) {
                    result.push_str(&value);
                } else {
                    // Variable not found, preserve original %VAR% token
                    result.push('%');
                    result.push_str(&var_name);
                    result.push('%');  // ← BUG: assumes closing % was found
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
```

**The bug**: Line 137's `take_while(|&c| c != '%')` doesn't distinguish between:
- Finding a `%` and stopping → closing delimiter found
- Reaching end of string → closing delimiter NOT found

Line 147 then blindly adds a closing `%` regardless.

## Bug Examples

### Example 1: Missing Closing Delimiter
**Input**: `"%PROGRAMFILES\Google\Chrome\chrome.exe"`

**What happens**:
1. Encounters first `%`
2. `take_while` consumes: `"PROGRAMFILES\Google\Chrome\chrome.exe"` (entire rest of string!)
3. Lookup fails (no env var named "PROGRAMFILES\Google\Chrome\chrome.exe")
4. Reconstructs: `"%PROGRAMFILES\Google\Chrome\chrome.exe%"` ← **WRONG** (added closing %)

**Expected**: `"%PROGRAMFILES\Google\Chrome\chrome.exe"` (preserve as-is)

### Example 2: Multiple Variables, Second Malformed
**Input**: `"%PROGRAMFILES%\%INVALID"`

**What happens**:
1. First `%PROGRAMFILES%` expands correctly (e.g., to `"C:\Program Files"`)
2. Second `%INVALID` has no closing `%`
3. `take_while` consumes: `"INVALID"` (hits end of string)
4. Reconstructs: `"C:\Program Files\%INVALID%"` ← **WRONG** (added closing %)

**Expected**: `"C:\Program Files\%INVALID"` (preserve malformed part as-is)

## Usage Context

**Call site**: [`src/browser_setup.rs:75-78`](../src/browser_setup.rs#L75-L78)
```rust
} else if path_str.contains('%') && cfg!(target_os = "windows") {
    // Expand environment variables on Windows (%VAR% tokens)
    let expanded = expand_windows_env_vars(path_str);
    PathBuf::from(expanded)
}
```

**Paths using environment variables**: [`src/browser_setup.rs:36-38`](../src/browser_setup.rs#L36-L38)
```rust
r"%PROGRAMFILES%\Google\Chrome\Application\chrome.exe",
r"%PROGRAMFILES(X86)%\Google\Chrome\Application\chrome.exe",
r"%LOCALAPPDATA%\Google\Chrome\Application\chrome.exe",
```

These hard-coded paths are well-formed (have closing `%`). The bug would only trigger if:
- User sets malformed `CHROMIUM_PATH` environment variable
- Future code adds malformed paths

## Impact Analysis

**Severity**: Medium
- **Scope**: Windows only (function only called on Windows)
- **Current risk**: Low (hard-coded paths are well-formed)
- **Latent risk**: Medium (user-provided paths could be malformed)
- **Consequence**: Chrome executable search fails → triggers browser download

**Real-world impact**:
- User with malformed `CHROMIUM_PATH` gets slow first launch
- Unexpected browser download (~100MB)
- But system remains functional (downloaded browser works)

## Solution: Explicit Closing Delimiter Detection

### Fixed Implementation

**Replace lines 130-159 in `src/browser_setup.rs`:**

```rust
fn expand_windows_env_vars(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            // Collect characters until next '%' or end of string
            let mut var_name = String::new();
            let mut found_closing = false;

            for c in chars.by_ref() {
                if c == '%' {
                    found_closing = true;
                    break;
                }
                var_name.push(c);
            }

            if found_closing && !var_name.is_empty() {
                // Try to expand the variable
                if let Ok(value) = std::env::var(&var_name) {
                    result.push_str(&value);
                } else {
                    // Variable not found, preserve original %VAR% token
                    result.push('%');
                    result.push_str(&var_name);
                    result.push('%');
                }
            } else if found_closing && var_name.is_empty() {
                // Empty %% sequence, treat as single %
                result.push('%');
            } else {
                // No closing % found - treat as literal text
                result.push('%');
                result.push_str(&var_name);
                // Don't add closing % since it wasn't in the input
            }
        } else {
            result.push(ch);
        }
    }

    result
}
```

## What Changes

### Key Differences

**Before (buggy)**:
```rust
let var_name: String = chars.by_ref().take_while(|&c| c != '%').collect();
// No way to know if we stopped at '%' or end-of-string

if !var_name.is_empty() {
    // ... always reconstructs with closing % ...
    result.push('%');
}
```

**After (fixed)**:
```rust
let mut found_closing = false;

for c in chars.by_ref() {
    if c == '%' {
        found_closing = true;  // ← Explicit tracking
        break;
    }
    var_name.push(c);
}

if found_closing && !var_name.is_empty() {
    // ... only add closing % if it was found ...
} else {
    // ... handle missing closing % case ...
    result.push('%');
    result.push_str(&var_name);
    // No closing % added
}
```

### Behavior Table

| Input | Before (buggy) | After (fixed) |
|-------|----------------|---------------|
| `"%PROGRAMFILES%\Chrome"` | Expands correctly | Expands correctly |
| `"%PROGRAMFILES\Chrome"` | `"%PROGRAMFILES\Chrome%"` ❌ | `"%PROGRAMFILES\Chrome"` ✅ |
| `"%%"` | `"%"` ✅ | `"%"` ✅ |
| `"C:\Chrome"` | `"C:\Chrome"` ✅ | `"C:\Chrome"` ✅ |
| `"%A%\%B"` | `"C:\Foo\%B%"` ❌ | `"C:\Foo\%B"` ✅ |

## Implementation Steps

**File**: `src/browser_setup.rs`

**Lines to modify**: 130-159

### Step 1: Replace the function

Delete the current implementation (lines 130-159) and replace with the fixed version above.

### Step 2: Verify compilation

```bash
cargo check
```

Should compile without errors.

### Step 3: Verify behavior

The function should now:
- ✅ Expand valid environment variables: `%PROGRAMFILES%` → `C:\Program Files`
- ✅ Preserve malformed tokens: `%PROGRAMFILES\Chrome` → `%PROGRAMFILES\Chrome` (no added `%`)
- ✅ Handle empty `%%` → `%`
- ✅ Pass through non-variable text unchanged

## Definition of Done

✅ `src/browser_setup.rs:130-159` - Function replaced with version that explicitly tracks `found_closing`
✅ Code uses explicit `for` loop instead of `take_while`
✅ Function handles three cases: `found_closing && !empty`, `found_closing && empty`, `!found_closing`
✅ No closing `%` added when `found_closing == false`
✅ Code compiles without errors
✅ Function signature unchanged (no breaking changes)

## Verification Approach

1. **Code inspection**: Read the modified function and verify `found_closing` is tracked and checked
2. **Compilation**: Run `cargo check` to ensure no syntax errors
3. **Manual verification**: Trace through the logic for the bug examples above

## Edge Cases Handled

The fixed implementation correctly handles:

1. ✅ **Valid variable**: `"%PROGRAMFILES%"` → expands
2. ✅ **Missing closing**: `"%PROGRAMFILES"` → preserves as-is
3. ✅ **Empty delimiter**: `"%%"` → becomes `"%"`
4. ✅ **Multiple variables**: `"%A%\%B%"` → both expand
5. ✅ **Mixed valid/invalid**: `"%A%\%B"` → first expands, second preserved
6. ✅ **No variables**: `"C:\Chrome"` → passes through
7. ✅ **Variable not found**: `"%NOTEXIST%"` → preserves `"%NOTEXIST%"`

## Related Code

- [`src/browser_setup.rs:32-64`](../src/browser_setup.rs#L32-L64) - Path definitions (Windows paths use env vars)
- [`src/browser_setup.rs:75-78`](../src/browser_setup.rs#L75-L78) - Call site in `find_browser_executable()`
- [`src/browser_setup.rs:16-29`](../src/browser_setup.rs#L16-L29) - CHROMIUM_PATH override handling

## Why This Matters

While the current hard-coded paths are well-formed, this is a **correctness bug** that should be fixed:

1. **Defensive coding**: User-provided `CHROMIUM_PATH` could be malformed
2. **Principle of least surprise**: Function should preserve input when it can't parse it
3. **Future-proofing**: Prevents issues if new paths are added
4. **Code quality**: Functions should handle edge cases correctly
