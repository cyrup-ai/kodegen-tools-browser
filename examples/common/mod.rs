//! Shared utilities for browser HTTP server examples
//!
//! This module spawns the local kodegen-browser HTTP server and connects to it.

use anyhow::{Context, Result};
use http::header::{HeaderMap, HeaderValue};
use kodegen_mcp_client::{
    KodegenClient, KodegenConnection, X_KODEGEN_CONNECTION_ID, X_KODEGEN_GITROOT, X_KODEGEN_PWD,
    create_streamable_client,
};
use rmcp::model::CallToolResult;
use std::path::{Path, PathBuf};
use std::sync::{Mutex as StdMutex, OnceLock};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, watch};
use std::sync::Arc;
use uuid::Uuid;

/// Browser HTTP server configuration
const HTTP_PORT: u16 = kodegen_config::PORT_BROWSER - 10000;
const BINARY_NAME: &str = "kodegen-browser";
const PACKAGE_NAME: &str = "kodegen_tools_browser";

/// HTTP server URL for browser examples
const HTTP_URL: &str = const_format::formatcp!("http://127.0.0.1:{}/mcp", kodegen_config::PORT_BROWSER - 10000);

/// Cached workspace root
static WORKSPACE_ROOT: OnceLock<PathBuf> = OnceLock::new();
static WORKSPACE_ROOT_INIT: StdMutex<()> = StdMutex::new(());

/// Find workspace root using cargo metadata
pub fn find_workspace_root() -> Result<&'static PathBuf> {
    if let Some(root) = WORKSPACE_ROOT.get() {
        return Ok(root);
    }

    let _lock = WORKSPACE_ROOT_INIT
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;

    if let Some(root) = WORKSPACE_ROOT.get() {
        return Ok(root);
    }

    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version=1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .context("Failed to execute cargo metadata")?;

    if !output.status.success() {
        anyhow::bail!(
            "cargo metadata failed (exit code: {:?})",
            output.status.code()
        );
    }

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Invalid JSON from cargo metadata")?;

    let workspace_root = metadata["workspace_root"]
        .as_str()
        .context("No workspace_root in metadata")?;

    let path = PathBuf::from(workspace_root);
    WORKSPACE_ROOT
        .set(path)
        .map_err(|_| anyhow::anyhow!("Failed to cache workspace root"))?;
    WORKSPACE_ROOT
        .get()
        .ok_or_else(|| anyhow::anyhow!("Failed to retrieve cached workspace root"))
}

/// Find git repository root by walking up from start directory
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Server process handle
#[must_use = "ServerHandle must be kept alive or explicitly shutdown"]
pub struct ServerHandle {
    child: Option<Child>,
}

impl ServerHandle {
    pub fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            eprintln!("🛑 Shutting down HTTP server...");

            #[cfg(unix)]
            {
                if let Some(pid) = child.id() {
                    let _ = Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .status()
                        .await;
                }
            }

            #[cfg(not(unix))]
            {
                let _ = child.kill().await;
            }

            match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => {
                    eprintln!(
                        "✅ Server shut down gracefully (exit code: {})",
                        status.code().unwrap_or(-1)
                    );
                }
                Ok(Err(e)) => {
                    eprintln!("⚠️  Error waiting for server: {e}");
                    let _ = child.kill().await;
                }
                Err(_) => {
                    eprintln!("⚠️  Server shutdown timeout, killing forcefully...");
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
        }
        Ok(())
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            eprintln!("⚠️  ServerHandle dropped without explicit shutdown, killing server...");
            let _ = child.start_kill();
        }
    }
}

/// Kill processes on specified port (gracefully with fallback)
#[cfg(unix)]
pub async fn cleanup_port(port: u16) -> Result<()> {
    use std::time::Duration;
    
    eprintln!("🧹 Checking for processes on port {port}...");

    // Step 1: Find PIDs on port using lsof
    let output = Command::new("lsof")
        .args(["-ti", &format!(":{port}")])
        .output()
        .await
        .context("Failed to run lsof")?;

    if !output.status.success() || output.stdout.is_empty() {
        eprintln!("   No processes found on port {port}");
        return Ok(());
    }

    // Step 2: Parse and validate PIDs
    let pids_string = String::from_utf8_lossy(&output.stdout);
    let pids: Vec<&str> = pids_string
        .lines()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if pids.is_empty() {
        return Ok(());
    }

    // Step 3: Gracefully shutdown each process
    for pid_str in pids {
        // Validate PID is numeric
        if pid_str.parse::<u32>().is_err() {
            eprintln!("   ⚠️  Invalid PID: {pid_str}, skipping");
            continue;
        }

        // Optional safety check: Verify process name looks like kodegen/cargo
        // This prevents accidentally killing unrelated processes
        let proc_check = Command::new("ps")
            .args(["-p", pid_str, "-o", "comm="])
            .output()
            .await;
        
        if let Ok(proc_output) = proc_check {
            let proc_name = String::from_utf8_lossy(&proc_output.stdout);
            let proc_name_trimmed = proc_name.trim();
            
            // Allow kodegen binaries and cargo (for development)
            if !proc_name_trimmed.contains("kodegen") 
                && !proc_name_trimmed.contains("cargo")
                && !proc_name_trimmed.is_empty() 
            {
                eprintln!(
                    "   ⚠️  Process {pid_str} ({proc_name_trimmed}) doesn't look like kodegen, skipping"
                );
                continue;
            }
        }

        eprintln!("   Sending SIGTERM to PID {pid_str}...");
        
        // Step 3a: Try graceful shutdown first (SIGTERM = signal 15)
        let term_result = Command::new("kill")
            .args(["-TERM", pid_str])
            .status()
            .await;
            
        if let Err(e) = term_result {
            eprintln!("   ⚠️  Failed to send SIGTERM to {pid_str}: {e}");
            continue;
        }

        // Step 3b: Wait up to 3 seconds for graceful exit
        // Poll every 500ms to check if process has exited
        let mut exited = false;
        for attempt in 0..6 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            // Check if process still exists using kill -0
            // Signal 0 doesn't actually send a signal, just checks if PID exists
            let check = Command::new("kill")
                .args(["-0", pid_str])
                .status()
                .await;
                
            if check.map(|s| !s.success()).unwrap_or(true) {
                eprintln!("   ✅ Process {pid_str} exited gracefully after {}ms", (attempt + 1) * 500);
                exited = true;
                break;
            }
        }

        // Step 3c: Force kill if still alive after grace period
        if !exited {
            eprintln!("   ⚠️  Process {pid_str} didn't exit gracefully, sending SIGKILL...");
            match Command::new("kill").args(["-9", pid_str]).status().await {
                Ok(status) if status.success() => {
                    eprintln!("   💀 Process {pid_str} killed with SIGKILL");
                }
                Ok(status) => {
                    eprintln!("   ⚠️  SIGKILL failed with exit code: {:?}", status.code());
                }
                Err(e) => {
                    eprintln!("   ⚠️  Failed to send SIGKILL to {pid_str}: {e}");
                }
            }
        }
    }

    // Step 4: Brief delay to ensure port is released by OS
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    Ok(())
}

#[cfg(not(unix))]
pub async fn cleanup_port(port: u16) -> Result<()> {
    eprintln!("⚠️  Port cleanup not implemented for this platform");
    eprintln!("   Please manually stop any process on port {port}");
    Ok(())
}

/// Classify connection error into diagnostic category
///
/// Examines error chain to determine the root cause category.
/// Uses pattern matching on error messages since we convert to anyhow::Error.
fn classify_connection_error(error: &anyhow::Error) -> String {
    let error_str = error.to_string().to_lowercase();
    let error_debug = format!("{:?}", error).to_lowercase();
    
    // Check for common connection error patterns
    // Order matters: more specific checks first
    
    if error_str.contains("connection refused") || error_debug.contains("connectionrefused") {
        "connection_refused".to_string()
    } else if error_str.contains("dns") 
        || error_str.contains("could not resolve") 
        || error_str.contains("name or service not known")
        || error_str.contains("nodename nor servname provided") {
        "dns_error".to_string()
    } else if error_str.contains("tls") 
        || error_str.contains("ssl") 
        || error_str.contains("certificate") 
        || error_str.contains("handshake") {
        "tls_error".to_string()
    } else if error_str.contains("connection closed") 
        || error_str.contains("transport closed")
        || error_debug.contains("connectionclosed") {
        "connection_closed".to_string()
    } else if error_str.contains("transport error") 
        || error_str.contains("transport send") {
        "transport_error".to_string()
    } else if error_str.contains("timeout") {
        "timeout".to_string()
    } else if error_str.contains("init") 
        || error_str.contains("initialization") {
        "init_error".to_string()
    } else if error_str.contains("protocol") 
        || error_str.contains("mcp") {
        "protocol_error".to_string()
    } else {
        // Fallback: use first word of error or "unknown"
        error_str
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .to_string()
    }
}

/// Connect to HTTP server with retry
pub async fn connect_with_retry(
    url: &str,
    total_timeout: std::time::Duration,
    retry_interval: std::time::Duration,
    mut server_child: Option<&mut Child>,
) -> Result<(KodegenClient, KodegenConnection)> {
    let start = std::time::Instant::now();
    let mut attempt = 0;
    let mut last_progress_log = start;
    
    // Track last error type to detect state transitions
    let mut last_error_type: Option<String> = None;

    // Build session headers
    let mut headers = HeaderMap::new();

    // Connection ID - unique per example run
    let connection_id = Uuid::new_v4().to_string();
    headers.insert(
        X_KODEGEN_CONNECTION_ID,
        HeaderValue::from_str(&connection_id).context("Failed to convert connection ID to header value")?,
    );

    // Current working directory
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    headers.insert(
        X_KODEGEN_PWD,
        HeaderValue::from_str(&cwd.to_string_lossy()).context("Failed to convert PWD to header value")?,
    );

    // Git root if available
    if let Some(git_root) = find_git_root(&cwd) {
        headers.insert(
            X_KODEGEN_GITROOT,
            HeaderValue::from_str(&git_root.to_string_lossy()).context("Failed to convert git root to header value")?,
        );
    }

    loop {
        attempt += 1;

        // Check if server process died
        if let Some(ref mut child) = server_child {
            match child.try_wait() {
                Ok(None) => {
                    // Process still running - continue with connection attempt
                }
                Ok(Some(status)) => {
                    // Process exited unexpectedly - fail fast with clear error
                    return Err(anyhow::anyhow!(
                        "Server process exited unexpectedly with status: {:?}. \
                         Possible causes: port conflict, configuration error, or panic. \
                         Check server logs for details.",
                        status
                    ));
                }
                Err(e) => {
                    // Error checking process status - treat as fatal
                    return Err(anyhow::anyhow!(
                        "Failed to check server process status: {}. \
                         The process may have been terminated externally.",
                        e
                    ));
                }
            }
        }

        match create_streamable_client(url, headers.clone()).await {
            Ok(result) => {
                eprintln!(
                    "✅ Connected to HTTP server in {:?} (attempt {})",
                    start.elapsed(),
                    attempt
                );
                return Ok(result);
            }
            Err(e) => {
                let error: anyhow::Error = e.into();
                
                // Classify error to detect state changes
                let error_type = classify_connection_error(&error);
                
                // Print error when type changes (indicates state transition)
                if last_error_type.as_ref() != Some(&error_type) {
                    // Format the category name for display
                    let category_display = error_type.replace('_', " ");
                    
                    eprintln!(
                        "   ⚠️  Connection error ({}): {}",
                        category_display,
                        error
                    );
                    
                    // Provide context for common expected errors
                    if error_type == "connection_refused" {
                        eprintln!("   (This is expected during server compilation, will keep retrying...)");
                    }
                    
                    last_error_type = Some(error_type.clone());
                }

                // Check if we've exceeded the total timeout
                if start.elapsed() >= total_timeout {
                    return Err(error.context(format!(
                        "Connection timeout after {} attempts over {:?}. Last error type: {}",
                        attempt,
                        start.elapsed(),
                        error_type
                    )));
                }

                // Progress logging every 10 seconds
                if last_progress_log.elapsed() >= std::time::Duration::from_secs(10) {
                    eprintln!(
                        "   Still waiting for server... ({:?} elapsed, {} attempts, current error: {})",
                        start.elapsed(),
                        attempt,
                        error_type.replace('_', " ")
                    );
                    last_progress_log = std::time::Instant::now();
                }

                tokio::time::sleep(retry_interval).await;
            }
        }
    }
}

/// Connect to local browser HTTP server
pub async fn connect_to_local_http_server() -> Result<(KodegenConnection, ServerHandle)> {
    let workspace_root = find_workspace_root().context("Failed to find workspace root")?;
    
    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 1: BUILD - Compile the binary explicitly
    // ═══════════════════════════════════════════════════════════════════════════
    eprintln!("🔨 Building {} (this may take 60-90s on first compile, 10-30s incremental)...", BINARY_NAME);
    
    let build_status = Command::new("cargo")
        .current_dir(workspace_root)
        .args([
            "build",
            "--package", PACKAGE_NAME,
            "--bin", BINARY_NAME,
            "--features", "server",
        ])
        .status()  // Wait for build to complete, returns exit status
        .await
        .context("Failed to execute cargo build")?;
    
    if !build_status.success() {
        anyhow::bail!(
            "cargo build failed with exit code: {:?}\n\
             Run manually to see compilation errors:\n  \
             cargo build --package {} --bin {} --features server",
            build_status.code(),
            PACKAGE_NAME,
            BINARY_NAME
        );
    }
    
    eprintln!("✅ Build complete");
    
    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 2: RUN - Execute the pre-built binary directly
    // ═══════════════════════════════════════════════════════════════════════════
    
    // Construct binary path: workspace_root/target/debug/kodegen-browser
    let binary_path = workspace_root.join("target").join("debug").join(BINARY_NAME);
    
    if !binary_path.exists() {
        anyhow::bail!(
            "Binary not found at expected path: {}\n\
             This should not happen after successful build.",
            binary_path.display()
        );
    }
    
    // Clean up any stale processes on the port
    cleanup_port(HTTP_PORT).await.ok();
    
    eprintln!("🚀 Starting {} HTTP server on port {}...", BINARY_NAME, HTTP_PORT);
    
    // Build command to run binary directly (no cargo overhead)
    let mut cmd = Command::new(&binary_path);
    cmd.args(["--http", &format!("127.0.0.1:{}", HTTP_PORT)]);
    
    // Pass through GITHUB_TOKEN if set
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        cmd.env("GITHUB_TOKEN", token);
    }
    
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    
    let mut child = cmd
        .spawn()
        .context("Failed to spawn HTTP server process")?;
    
    // Forward stdout with [SERVER] prefix
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("[SERVER] {}", line);
            }
        });
    }
    
    // Forward stderr with [SERVER] prefix
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("[SERVER] {}", line);
            }
        });
    }
    
    // Server should start in 2-5 seconds (no compilation), so 30s timeout is generous
    eprintln!("⏳ Waiting for server to be ready (should be <5 seconds)...");
    let (_client, connection) = connect_with_retry(
        HTTP_URL,
        std::time::Duration::from_secs(30),    // Reduced from 180s
        std::time::Duration::from_millis(200), // Faster retry interval
        Some(&mut child),  // Monitor child during retry
    )
    .await
    .context(
        "Failed to connect to HTTP server.\n\
         Server started but failed to respond on port.\n\
         Check server logs for startup errors."
    )?;
    
    let server_handle = ServerHandle::new(child);
    
    Ok((connection, server_handle))
}

/// JSONL log entry
#[derive(Debug, serde::Serialize)]
pub struct LogEntry {
    timestamp: String,
    tool: String,
    args: serde_json::Value,
    duration_ms: u64,
    #[serde(flatten)]
    result: LogResult,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum LogResult {
    Success { response: serde_json::Value },
    Error { error: String },
}

/// Logging wrapper for KodegenClient
pub struct LoggingClient {
    inner: KodegenClient,
    log_file: Arc<Mutex<BufWriter<tokio::fs::File>>>,
    shutdown_tx: watch::Sender<bool>,
}

impl LoggingClient {
    pub async fn new(client: KodegenClient, log_path: impl AsRef<Path>) -> Result<Self> {
        // Create log directory if needed
        if let Some(parent) = log_path.as_ref().parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create log directory")?;
        }

        // Open log file with BufWriter (8KB buffer)
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(log_path)
            .await
            .context("Failed to open log file")?;

        let log_file = Arc::new(Mutex::new(BufWriter::new(file)));

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Spawn background flusher task
        Self::spawn_background_flusher(Arc::clone(&log_file), shutdown_rx);

        Ok(Self {
            inner: client,
            log_file,
            shutdown_tx,
        })
    }

    /// Spawn background task that periodically flushes buffered writes
    fn spawn_background_flusher(
        log_file: Arc<Mutex<BufWriter<tokio::fs::File>>>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) {
        tokio::spawn(async move {
            // Flush interval: 100ms (balances responsiveness vs. I/O efficiency)
            // Note: edit_log.rs and usage_tracker.rs use 5s, but browser operations
            // are more latency-sensitive and 100ms is still 10x better than per-entry
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    // Periodic flush
                    _ = interval.tick() => {
                        // Use try_lock to avoid blocking if write is in progress
                        if let Ok(mut guard) = log_file.try_lock() {
                            // Ignore flush errors - this is best-effort async I/O
                            let _ = guard.flush().await;
                        }
                        // If lock is held, skip this flush - will catch it next tick
                    }

                    // Shutdown signal received
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            // Final flush before shutdown
                            let mut guard = log_file.lock().await;
                            let _ = guard.flush().await;
                            break;
                        }
                    }
                }
            }
        });
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, kodegen_mcp_client::ClientError> {
        let start = tokio::time::Instant::now();
        let result = self.inner.call_tool(name, arguments.clone()).await;
        let duration = start.elapsed();

        self.log_call(name, arguments, &result, duration).await;
        result
    }

    async fn log_call(
        &self,
        name: &str,
        args: serde_json::Value,
        result: &Result<CallToolResult, kodegen_mcp_client::ClientError>,
        duration: std::time::Duration,
    ) {
        let log_result = match result {
            Ok(r) => {
                let response = serde_json::to_value(r)
                    .unwrap_or_else(|_| serde_json::json!({"serialization_error": true}));
                LogResult::Success { response }
            }
            Err(e) => LogResult::Error {
                error: e.to_string(),
            },
        };

        self.log_entry(name, args, log_result, duration).await;
    }

    async fn log_entry(
        &self,
        name: &str,
        args: serde_json::Value,
        result: LogResult,
        duration: std::time::Duration,
    ) {
        let entry = LogEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            tool: name.to_string(),
            args,
            duration_ms: duration.as_millis() as u64,
            result,
        };

        if let Err(e) = self.write_log_entry(&entry).await {
            eprintln!("⚠️  Failed to write log entry: {e}");
        }
    }

    async fn write_log_entry(&self, entry: &LogEntry) -> Result<()> {
        let json = serde_json::to_string(entry).context("Failed to serialize log entry")?;

        let mut guard = self.log_file.lock().await;
        guard
            .write_all(json.as_bytes())
            .await
            .context("Failed to write log entry")?;
        guard
            .write_all(b"\n")
            .await
            .context("Failed to write newline")?;
        
        // ✅ NO FLUSH - rely on BufWriter's 8KB buffer + background flusher
        // Flush happens automatically when:
        // 1. Buffer fills (8KB)
        // 2. Background task flushes (every 100ms)
        // 3. Drop/shutdown triggers final flush

        Ok(())
    }

    /// Manually flush buffered log entries to disk
    /// 
    /// This is optional - the background flusher handles periodic flushes.
    /// Use this before critical operations if you need guaranteed persistence.
    pub async fn flush(&self) -> Result<()> {
        let mut guard = self.log_file.lock().await;
        guard.flush().await.context("Failed to flush log")?;
        Ok(())
    }
}

impl Drop for LoggingClient {
    fn drop(&mut self) {
        // Signal background task to shutdown and perform final flush
        let _ = self.shutdown_tx.send(true);
        
        // Note: We can't await in Drop, but the background task will flush
        // before terminating. The tokio runtime ensures spawned tasks complete
        // during graceful shutdown.
    }
}
