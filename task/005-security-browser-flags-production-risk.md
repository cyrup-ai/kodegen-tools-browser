# Security Risk: Browser Launch Flags in Production

## Priority: MEDIUM-HIGH

## Location
`src/browser_setup.rs:243-276`

## Issue Description
The browser is launched with security-disabling flags including `--no-sandbox` and `--disable-web-security`. While these are common for automation, they pose security risks in production environments where untrusted content might be processed.

## Code Reference
```rust
// Add stealth mode arguments
config_builder = config_builder
    .arg(format!("--user-agent={}", CHROME_USER_AGENT))
    .arg("--disable-blink-features=AutomationControlled")
    .arg("--disable-infobars")
    // ... benign flags ...
    .arg("--disable-web-security")  // Line 252 - SECURITY RISK
    .arg("--disable-features=IsolateOrigins,site-per-process")  // Line 253 - SECURITY RISK
    .arg("--disable-setuid-sandbox")  // Line 254 - SECURITY RISK
    .arg("--no-first-run")
    .arg("--no-default-browser-check")
    .arg("--no-sandbox")  // Line 257 - SECURITY RISK
    .arg("--ignore-certificate-errors")  // Line 258 - SECURITY RISK
    // ... more flags ...
```

## Security Concerns

### Critical Flags

**`--no-sandbox` (Line 257)**
- **Risk**: Disables Chrome's process sandboxing
- **Impact**: Malicious content can escape browser and access system
- **Use Case**: Required in some containerized environments (Docker)
- **Mitigation**: Should be conditional, not default

**`--disable-web-security` (Line 252)**
- **Risk**: Disables Same-Origin Policy (SOP)
- **Impact**: XSS and CSRF protections disabled
- **Use Case**: Allows cross-origin requests for automation
- **Mitigation**: Only needed for specific use cases

**`--disable-features=IsolateOrigins,site-per-process` (Line 253)**
- **Risk**: Disables site isolation security feature
- **Impact**: Spectre/Meltdown mitigation disabled
- **Use Case**: Reduces memory overhead
- **Mitigation**: Should be optional for high-security environments

**`--disable-setuid-sandbox` (Line 254)**
- **Risk**: Disables setuid sandbox
- **Impact**: Reduces defense-in-depth on Linux
- **Use Case**: Avoid setuid binary requirement
- **Mitigation**: Often not needed with proper setup

**`--ignore-certificate-errors` (Line 258)**
- **Risk**: Accepts invalid SSL/TLS certificates
- **Impact**: MITM attacks possible
- **Use Case**: Testing with self-signed certs
- **Mitigation**: Should be optional

## Current Behavior

**All flags are hard-coded** - No way to disable them without code changes.

## Real-World Attack Scenarios

### Scenario 1: Malicious Search Results
1. User performs web search via `web_search` tool
2. Malicious site in results contains exploit
3. No sandbox → Exploit can access host filesystem
4. Agent-based research could visit many untrusted sites

### Scenario 2: User-Provided URLs
1. User asks agent to research a topic
2. Agent navigates to user-provided or discovered URLs
3. Malicious JavaScript exploits disabled security
4. Compromise of host system

### Scenario 3: MITM Attack
1. Network compromised or using public WiFi
2. HTTPS downgrade attack
3. `--ignore-certificate-errors` accepts invalid cert
4. Credentials or sensitive data leaked

## Impact on Production

**Risk Level**: Medium-High
- **Scope**: All browser launches
- **Frequency**: Every time browser starts
- **Likelihood**: Depends on trust level of visited sites
- **Impact**: Potential system compromise

**Affected Use Cases**:
- ✅ **Safe**: Visiting trusted sites (docs.rs, github.com)
- ⚠️ **Moderate Risk**: General web search (DuckDuckGo results)
- 🔴 **High Risk**: User-provided URLs, deep research on unknown sites
- 🔴 **High Risk**: Agent autonomous browsing

## Current Usage Context

From README.md, the tool is used for:
1. Basic navigation (user-controlled)
2. Web search (DuckDuckGo - some trust)
3. **Autonomous agent** (visits unknown sites)
4. **Background research** (visits many sites automatically)

Items #3 and #4 are high-risk with current flags.

## Recommended Fix

### Option 1: Make Security Flags Configurable (Recommended)

Add to `config.yaml`:
```yaml
browser:
  headless: true
  disable_security: false  # Already exists but ignored!
  sandbox: true            # NEW - default to secure
  strict_ssl: true         # NEW - default to secure
```

Then in code:
```rust
// Only disable security if explicitly requested
if config.disable_security {
    config_builder = config_builder
        .arg("--disable-web-security")
        .arg("--disable-features=IsolateOrigins,site-per-process");
}

// Only disable sandbox if explicitly requested
if !config.sandbox {
    config_builder = config_builder
        .arg("--no-sandbox")
        .arg("--disable-setuid-sandbox");
}

// Only ignore SSL errors if explicitly requested
if !config.strict_ssl {
    config_builder = config_builder
        .arg("--ignore-certificate-errors");
}
```

**Note**: `config.yaml` already has `disable_security: false` but it's currently ignored!

### Option 2: Remove Dangerous Flags Entirely

Remove these flags and only add them back if users explicitly need them:
- `--no-sandbox` (keep for Docker, but detect environment)
- `--disable-web-security` (rarely needed for read-only automation)
- `--ignore-certificate-errors` (almost never needed)

### Option 3: Environment-Based Detection

Detect Docker/container environments and only disable sandbox there:
```rust
fn should_disable_sandbox() -> bool {
    // Check if running in Docker
    std::path::Path::new("/.dockerenv").exists() ||
    std::env::var("container").is_ok()
}
```

## Compatibility Considerations

**Breaking Change Risk**: Some users may depend on these flags
- Docker users need `--no-sandbox`
- Some networks need `--ignore-certificate-errors`

**Migration Path**:
1. Add configuration options (default to secure)
2. Deprecation warning for 1 version
3. Change defaults to secure

## Related Code
- `src/lib.rs` - Config loading (should wire up disable_security flag)
- `config.yaml:3` - `disable_security: false` is already defined but unused!
- README.md - Documents the security risks are "disabled"

## Priority Justification

**Medium-High** because:
- Security impact is high if exploited
- But requires visiting malicious sites
- Config option already exists (just not wired up)
- Relatively easy fix
