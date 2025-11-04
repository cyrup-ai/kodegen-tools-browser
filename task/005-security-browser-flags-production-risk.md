# Security Risk: Browser Launch Flags in Production

## Priority: MEDIUM-HIGH

## Core Objective

**Wire up the existing `config.yaml` browser security settings to the browser launch process** so that security-critical flags (`--no-sandbox`, `--disable-web-security`, `--ignore-certificate-errors`) are configurable rather than hard-coded, allowing users to run in secure mode by default.

## The Problem

The browser is launched with hard-coded security-disabling flags that pose risks when processing untrusted content. **Critically, `config.yaml` already defines `disable_security: false`, but this setting is completely ignored by the Rust code.**

## Current State Analysis

### What Already Exists

**1. Configuration File** ([`config.yaml:1-11`](../config.yaml#L1-L11))
```yaml
# Browser Settings
browser:
  headless: true                    # If true, the browser runs in headless mode
  disable_security: false           # If true, disables security features in the browser
  window:
    width: 1280                     # Browser window width
    height: 720                     # Browser window height
```
✅ Config file already has browser settings structure
✅ `disable_security: false` already defined (secure by default)
❌ **NOT loaded or used by Rust code**

**2. Config Struct** ([`src/lib.rs:21-34`](../src/lib.rs#L21-L34))
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,

    #[serde(default = "default_max_steps")]
    pub max_steps: usize,

    #[serde(default = "default_search_engine")]
    pub search_engine: String,
}
```
❌ **Missing browser configuration fields**
❌ Does not load `browser:` section from YAML

**3. Browser Launch Function** ([`src/browser_setup.rs:209-297`](../src/browser_setup.rs#L209-L297))
```rust
pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
) -> Result<(Browser, JoinHandle<()>)> {
    // ... setup code ...

    // Hard-coded security flags (lines 243-276)
    config_builder = config_builder
        .arg("--disable-web-security")              // Line 252 - SECURITY RISK
        .arg("--disable-features=IsolateOrigins,site-per-process")  // Line 253 - SECURITY RISK
        .arg("--disable-setuid-sandbox")            // Line 254 - SECURITY RISK
        .arg("--no-sandbox")                        // Line 257 - SECURITY RISK
        .arg("--ignore-certificate-errors");        // Line 258 - SECURITY RISK

    // ... more hard-coded flags ...
}
```
❌ All security flags hard-coded
❌ No config parameter accepted

## Security Impact

### Critical Hard-Coded Flags

| Flag | Line | Risk | Impact |
|------|------|------|--------|
| `--no-sandbox` | 257 | HIGH | Malicious content can escape browser, access system |
| `--disable-web-security` | 252 | HIGH | XSS/CSRF protections disabled, Same-Origin Policy bypassed |
| `--ignore-certificate-errors` | 258 | MEDIUM | MITM attacks possible, invalid certs accepted |
| `--disable-features=IsolateOrigins` | 253 | MEDIUM | Spectre/Meltdown mitigation disabled |
| `--disable-setuid-sandbox` | 254 | LOW | Reduces Linux defense-in-depth |

### Real-World Attack Scenarios

**Scenario 1: Agent Visits Malicious Site**
1. Autonomous agent researches topic
2. Discovers malicious site in search results
3. Navigates to site (no sandbox protection)
4. Exploit code runs with full system access
5. Host system compromised

**Scenario 2: MITM on Public WiFi**
1. User on public network
2. MITM intercepts HTTPS
3. `--ignore-certificate-errors` accepts forged certificate
4. Credentials/data stolen

## Solution: Wire Up Existing Config

### Implementation Path

The config flow needs to be:
```
config.yaml
  ↓ (load_yaml_config)
src/lib.rs::Config
  ↓ (pass to manager)
src/manager.rs::get_or_launch
  ↓ (pass to wrapper)
src/browser/wrapper.rs::launch_browser
  ↓ (pass to setup)
src/browser_setup.rs::launch_browser
  ↓ (conditionally add flags)
Chrome launch
```

### Step 1: Extend Config Struct

**File: `src/lib.rs`**

**Current (lines 21-34):**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,

    #[serde(default = "default_max_steps")]
    pub max_steps: usize,

    #[serde(default = "default_search_engine")]
    pub search_engine: String,
}
```

**Add after line 34:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,

    #[serde(default = "default_max_steps")]
    pub max_steps: usize,

    #[serde(default = "default_search_engine")]
    pub search_engine: String,

    // NEW: Browser configuration
    #[serde(default)]
    pub browser: BrowserConfig,
}

/// Browser security and launch configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    /// Run browser in headless mode
    #[serde(default = "default_headless")]
    pub headless: bool,

    /// Disable web security features (Same-Origin Policy, etc.)
    /// WARNING: Only enable for trusted content
    #[serde(default = "default_disable_security")]
    pub disable_security: bool,

    /// Window dimensions
    #[serde(default)]
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    #[serde(default = "default_window_width")]
    pub width: u32,

    #[serde(default = "default_window_height")]
    pub height: u32,
}

// Default functions (add after line 47)
fn default_headless() -> bool {
    true
}

fn default_disable_security() -> bool {
    false  // SECURE BY DEFAULT
}

fn default_window_width() -> u32 {
    1280
}

fn default_window_height() -> u32 {
    720
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            headless: default_headless(),
            disable_security: default_disable_security(),
            window: WindowConfig::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: default_window_width(),
            height: default_window_height(),
        }
    }
}
```

### Step 2: Modify browser_setup::launch_browser

**File: `src/browser_setup.rs`**

**Current signature (line 209):**
```rust
pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
) -> Result<(Browser, JoinHandle<()>)>
```

**Change to:**
```rust
pub async fn launch_browser(
    headless: bool,
    chrome_data_dir: Option<PathBuf>,
    disable_security: bool,
) -> Result<(Browser, JoinHandle<()>)>
```

**Modify security flags section (lines 243-276):**

**Current (hard-coded):**
```rust
// Add stealth mode arguments
config_builder = config_builder
    .arg(format!("--user-agent={}", CHROME_USER_AGENT))
    .arg("--disable-blink-features=AutomationControlled")
    .arg("--disable-infobars")
    .arg("--disable-notifications")
    .arg("--disable-print-preview")
    .arg("--disable-desktop-notifications")
    .arg("--disable-software-rasterizer")
    .arg("--disable-web-security")                              // ← CONDITIONAL
    .arg("--disable-features=IsolateOrigins,site-per-process")  // ← CONDITIONAL
    .arg("--disable-setuid-sandbox")                            // ← CONDITIONAL
    .arg("--no-first-run")
    .arg("--no-default-browser-check")
    .arg("--no-sandbox")                                        // ← CONDITIONAL
    .arg("--ignore-certificate-errors")                         // ← CONDITIONAL
    .arg("--enable-features=NetworkService,NetworkServiceInProcess")
    // Additional stealth arguments
    .arg("--disable-extensions")
    .arg("--disable-popup-blocking")
    // ... more benign flags ...
```

**Replace with (conditional flags):**
```rust
// Add stealth mode arguments (benign flags always added)
config_builder = config_builder
    .arg(format!("--user-agent={}", CHROME_USER_AGENT))
    .arg("--disable-blink-features=AutomationControlled")
    .arg("--disable-infobars")
    .arg("--disable-notifications")
    .arg("--disable-print-preview")
    .arg("--disable-desktop-notifications")
    .arg("--disable-software-rasterizer")
    .arg("--no-first-run")
    .arg("--no-default-browser-check")
    .arg("--enable-features=NetworkService,NetworkServiceInProcess")
    // Additional stealth arguments (benign)
    .arg("--disable-extensions")
    .arg("--disable-popup-blocking")
    .arg("--disable-background-networking")
    .arg("--disable-background-timer-throttling")
    .arg("--disable-backgrounding-occluded-windows")
    .arg("--disable-breakpad")
    .arg("--disable-component-extensions-with-background-pages")
    .arg("--disable-features=TranslateUI")
    .arg("--disable-hang-monitor")
    .arg("--disable-ipc-flooding-protection")
    .arg("--disable-prompt-on-repost")
    .arg("--metrics-recording-only")
    .arg("--password-store=basic")
    .arg("--use-mock-keychain")
    .arg("--hide-scrollbars")
    .arg("--mute-audio");

// Conditionally add security-disabling flags
if disable_security {
    info!("WARNING: Disabling browser security features (disable_security=true)");
    config_builder = config_builder
        .arg("--disable-web-security")
        .arg("--disable-features=IsolateOrigins,site-per-process")
        .arg("--ignore-certificate-errors");
}

// Always disable sandbox in containerized environments (Docker detection)
if should_disable_sandbox() {
    info!("Detected containerized environment, disabling sandbox");
    config_builder = config_builder
        .arg("--no-sandbox")
        .arg("--disable-setuid-sandbox");
} else if disable_security {
    // Only disable sandbox if explicitly requested AND not in container
    config_builder = config_builder
        .arg("--no-sandbox")
        .arg("--disable-setuid-sandbox");
}
```

**Add helper function (after line 297):**
```rust
/// Detect if running in containerized environment (Docker, etc.)
/// In containers, sandbox must be disabled as setuid doesn't work
fn should_disable_sandbox() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::env::var("container").is_ok()
        || std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
}
```

### Step 3: Update browser/wrapper.rs

**File: `src/browser/wrapper.rs`**

**Current (line 104-117):**
```rust
pub async fn launch_browser() -> Result<(Browser, JoinHandle<()>, PathBuf)> {
    info!("Launching main browser instance");

    let user_data_dir = std::env::temp_dir().join(format!("kodegen_browser_main_{}", std::process::id()));

    let (browser, handler) = crate::browser_setup::launch_browser(
        true, // headless
        Some(user_data_dir.clone())
    ).await?;

    Ok((browser, handler, user_data_dir))
}
```

**Change to:**
```rust
pub async fn launch_browser() -> Result<(Browser, JoinHandle<()>, PathBuf)> {
    info!("Launching main browser instance");

    // Load configuration
    let config = crate::load_yaml_config().unwrap_or_default();

    let user_data_dir = std::env::temp_dir().join(format!("kodegen_browser_main_{}", std::process::id()));

    let (browser, handler) = crate::browser_setup::launch_browser(
        config.browser.headless,
        Some(user_data_dir.clone()),
        config.browser.disable_security,  // NEW: Pass security config
    ).await?;

    Ok((browser, handler, user_data_dir))
}
```

### Step 4: Update config.yaml (Already Correct!)

**File: `config.yaml`**

The config.yaml already has the correct structure - no changes needed!
```yaml
browser:
  headless: true
  disable_security: false  # ✅ Secure by default
  window:
    width: 1280
    height: 720
```

## What Changes

### Files Modified

1. **`src/lib.rs`** (lines 21-58)
   - Add `BrowserConfig` struct with `disable_security` field
   - Add `WindowConfig` struct
   - Add default functions
   - Update `Config` struct to include `browser: BrowserConfig`

2. **`src/browser_setup.rs`** (lines 209, 243-276)
   - Add `disable_security: bool` parameter to `launch_browser()`
   - Make security flags conditional on `disable_security`
   - Add `should_disable_sandbox()` helper for Docker detection
   - Keep benign stealth flags always enabled

3. **`src/browser/wrapper.rs`** (lines 104-117)
   - Load config via `load_yaml_config()`
   - Pass `config.browser.headless` instead of hard-coded `true`
   - Pass `config.browser.disable_security` to `browser_setup::launch_browser()`

### Flags Categorization

**Always Safe (never conditional):**
- `--disable-blink-features=AutomationControlled` (stealth)
- `--disable-infobars` (UI cleanup)
- `--disable-extensions` (performance)
- `--hide-scrollbars` (visual)
- `--mute-audio` (convenience)

**Conditional on `disable_security=true`:**
- `--disable-web-security` (Same-Origin Policy bypass)
- `--disable-features=IsolateOrigins,site-per-process` (site isolation)
- `--ignore-certificate-errors` (SSL validation)
- `--no-sandbox` (unless in Docker)
- `--disable-setuid-sandbox` (unless in Docker)

**Auto-detected (Docker/container):**
- `--no-sandbox` (required in containers)
- `--disable-setuid-sandbox` (required in containers)

## Definition of Done

✅ `src/lib.rs` - Added `BrowserConfig` and `WindowConfig` structs with proper serde derives
✅ `src/lib.rs` - Updated `Config` struct to include `browser: BrowserConfig` field
✅ `src/browser_setup.rs` - Added `disable_security` parameter to `launch_browser()`
✅ `src/browser_setup.rs` - Made 5 security-critical flags conditional
✅ `src/browser_setup.rs` - Added `should_disable_sandbox()` for Docker detection
✅ `src/browser/wrapper.rs` - Load config and pass `disable_security` to browser_setup
✅ Code compiles without errors
✅ Default behavior is SECURE (`disable_security: false` in config.yaml)
✅ Users can opt-in to relaxed security by setting `disable_security: true`

## Verification Approach

1. **Default secure mode:**
   - Run with default `config.yaml` (disable_security: false)
   - Verify `--no-sandbox`, `--disable-web-security` NOT in launch args
   - Verify in Docker: sandbox flags still added (auto-detected)

2. **Opt-in insecure mode:**
   - Set `disable_security: true` in config.yaml
   - Verify security flags ARE added to launch args

3. **Config loading:**
   - Verify config.yaml browser section loads correctly
   - Verify defaults apply if config.yaml missing

## References

### Current Code

- [`config.yaml:4-10`](../config.yaml#L4-L10) - Existing browser config (unused)
- [`src/lib.rs:21-71`](../src/lib.rs#L21-L71) - Config struct (incomplete)
- [`src/browser_setup.rs:209-297`](../src/browser_setup.rs#L209-L297) - Browser launch with hard-coded flags
- [`src/browser/wrapper.rs:104-117`](../src/browser/wrapper.rs#L104-L117) - Wrapper launch function

### Security Context

**Why these flags are dangerous:**
- `--no-sandbox`: Chrome process sandboxing is primary security boundary
- `--disable-web-security`: Same-Origin Policy is fundamental web security model
- `--ignore-certificate-errors`: Certificate validation prevents MITM attacks
- Site isolation: Protects against Spectre/Meltdown side-channel attacks

**When they might be needed:**
- `--no-sandbox`: Docker/containers where setuid doesn't work
- `--disable-web-security`: Rare - cross-origin dev/testing only
- `--ignore-certificate-errors`: Dev environments with self-signed certs
- **Production use cases are RARE** - secure by default is correct

## Migration Notes

**Breaking Change:** None - defaults to current secure behavior
**Compatibility:** Users who need insecure mode can opt-in via config
**Docker/Containers:** Auto-detected, sandbox disabled automatically
