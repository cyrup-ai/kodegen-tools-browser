# Bug in expand_windows_env_vars: Malformed Input Handling

## Priority: MEDIUM

## Location
`src/browser_setup.rs:130-159`

## Issue Description
The `expand_windows_env_vars` function doesn't properly handle malformed input where environment variable tokens are missing their closing `%`. This causes the function to silently consume the rest of the string, leading to incorrect path expansion.

## Code Reference
```rust
fn expand_windows_env_vars(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            // Found start of potential environment variable
            let var_name: String = chars.by_ref().take_while(|&c| c != '%').collect();  // Line 137

            if !var_name.is_empty() {
                // Try to expand the variable
                if let Ok(value) = std::env::var(&var_name) {
                    result.push_str(&value);
                } else {
                    // Variable not found, preserve original %VAR% token
                    result.push('%');
                    result.push_str(&var_name);
                    result.push('%');  // Line 147 - BUG: assumes closing % was found
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

## Problem Details

1. **Unchecked Assumption**: Line 137 uses `take_while(|&c| c != '%')` which consumes characters until it finds a `%` or reaches end of string
2. **No Closing % Detection**: The code doesn't check if a closing `%` was actually found
3. **Silent Corruption**: If no closing `%` exists, the entire rest of the string becomes the variable name
4. **Incorrect Reconstruction**: Line 147 always adds a closing `%` even if one wasn't present in the input

## Example Bug Scenarios

**Input**: `"%PROGRAMFILES\Google\Chrome\chrome.exe"`
- Missing closing `%` after PROGRAMFILES
- `take_while` consumes: `"PROGRAMFILES\Google\Chrome\chrome.exe"`
- Lookup fails for this long "variable name"
- Output: `"%PROGRAMFILES\Google\Chrome\chrome.exe%"` (incorrect - added closing %)

**Input**: `"%PROGRAMFILES%\%INVALID"`
- Second `%` has no closing
- First variable expands correctly
- Second variable consumes rest of string
- Output corruption

## Current Usage
The function is called from line 77 in the same file:
```rust
} else if path_str.contains('%') && cfg!(target_os = "windows") {
    // Expand environment variables on Windows (%VAR% tokens)
    let expanded = expand_windows_env_vars(path_str);
    PathBuf::from(expanded)
}
```

This is used when searching for Chrome executables in predefined paths (lines 32-64).

## Impact on Production

**Severity**: Medium
- **Scope**: Windows only
- **Frequency**: Only when CHROMIUM_PATH or predefined paths contain malformed `%` tokens
- **Consequence**: Chrome executable won't be found, falls back to download
- **User Impact**: Slower first launch, unexpected browser downloads

**Real-World Likelihood**: Low
- Predefined paths are hard-coded and correct
- Would only trigger if user sets malformed CHROMIUM_PATH
- However, it's still a latent bug

## Recommended Fix

Add explicit detection of whether the closing `%` was found:

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
                // Empty %% sequence
                result.push('%');
            } else {
                // No closing % found - treat as literal
                result.push('%');
                result.push_str(&var_name);
                // Don't add closing % since it wasn't there
            }
        } else {
            result.push(ch);
        }
    }

    result
}
```

## Testing Recommendations

Add unit tests for edge cases:
1. Valid: `"%PROGRAMFILES%\Chrome"` → expands correctly
2. Malformed: `"%PROGRAMFILES\Chrome"` → preserves `"%PROGRAMFILES\Chrome"` (no closing %)
3. Empty: `"%%"` → `"%"`
4. Multiple: `"%A%\%B%"` → expands both if they exist
5. Nested (unsupported): `"%A%B%"` → handles gracefully
6. No variables: `"C:\Chrome"` → passes through unchanged

## Related Code
- `src/browser_setup.rs:32-64` - Path definitions using this function
- `src/browser_setup.rs:75-81` - Call site
