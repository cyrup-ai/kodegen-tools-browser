<div align="center">
  <img src="assets/img/banner.png" alt="Kodegen AI Banner" width="100%" />
</div>

# kodegen-tools-browser

[![License](https://img.shields.io/badge/license-Apache%202.0%20OR%20MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-nightly-orange.svg)](https://www.rust-lang.org)

**Memory-efficient, blazing-fast MCP tools for browser automation and AI-powered web research.**

kodegen-tools-browser is a high-performance Model Context Protocol (MCP) server that provides browser automation capabilities for AI agents. Built in Rust with chromiumoxide, it offers 13 specialized tools for web automation, search, and autonomous research.

## Features

- **🚀 High Performance**: Rust-powered with global singleton browser instance (~2-3s first launch, <1ms subsequent calls)
- **🎭 Stealth Automation**: Kromekover evasion system with 20+ anti-detection scripts
- **🤖 Autonomous Agent**: LLM-powered browser navigation with local inference
- **🔍 Deep Research**: Background async research sessions with progress tracking
- **🌐 Web Search**: Integrated DuckDuckGo search with result extraction
- **📸 Rich Capture**: Screenshots, text extraction, and DOM manipulation
- **⚡ MCP Native**: Serves tools over HTTP/SSE transport for AI agent integration

## Installation

### Prerequisites 

- Rust nightly toolchain
- Google Chrome (auto-downloaded if not found)
- macOS, Linux, or Windows

### Build from Source

```bash
# Clone the repository
git clone https://github.com/cyrup-ai/kodegen-tools-browser.git
cd kodegen-tools-browser

# Install Rust nightly (if not already installed)
rustup toolchain install nightly
rustup default nightly

# Build the project
cargo build --release

# Run the server
cargo run --release --bin kodegen-browser
```

### Configuration

Edit `config.yaml` to customize browser behavior:

```yaml
browser:
  headless: true                    # Run browser in headless mode
  disable_security: false           # Security features (keep enabled)
  window:
    width: 1280                     # Browser window width
    height: 720                     # Browser window height
```

## Available Tools

### Core Automation (6 tools)

| Tool | Description |
|------|-------------|
| `browser_navigate` | Navigate to URL with optional selector wait |
| `browser_click` | Click elements with navigation wait support |
| `browser_type_text` | Type text into input fields |
| `browser_extract_text` | Extract visible text or selector content |
| `browser_scroll` | Scroll page by pixels or to selector |
| `browser_screenshot` | Capture page or element screenshots |

### Web Search (1 tool)

| Tool | Description |
|------|-------------|
| `web_search` | DuckDuckGo search with structured result extraction |

### Async Research (5 tools)

| Tool | Description |
|------|-------------|
| `start_browser_research` | Start background research session |
| `get_research_status` | Poll session progress |
| `get_research_result` | Retrieve completed research |
| `stop_browser_research` | Cancel running session |
| `list_research_sessions` | List all active sessions |

### Autonomous Agent (1 tool)

| Tool | Description |
|------|-------------|
| `browser_agent` | AI-powered autonomous task execution with local LLM |

## Quick Start

### Example 1: Basic Web Automation

```rust
use kodegen_tools_browser::{BrowserManager, BrowserNavigateTool};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    // Get global browser instance
    let manager = BrowserManager::global();

    // Navigate to a URL
    let tool = BrowserNavigateTool::new(manager.clone());
    tool.execute(json!({
        "url": "https://docs.rs",
        "wait_for_selector": "#search"
    })).await?;

    Ok(())
}
```

### Example 2: Web Search

```rust
use kodegen_tools_browser::WebSearchTool;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let tool = WebSearchTool::new();
    let result = tool.execute(json!({
        "query": "Rust MCP server examples"
    })).await?;

    println!("Search results: {:?}", result);
    Ok(())
}
```

### Example 3: Autonomous Research

```rust
use kodegen_tools_browser::StartBrowserResearchTool;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let tool = StartBrowserResearchTool::new();

    // Start background research
    let result = tool.execute(json!({
        "query": "Latest Rust async runtime benchmarks",
        "max_pages": 5
    })).await?;

    let session_id = result["session_id"].as_str().unwrap();
    println!("Research session started: {}", session_id);

    // Poll for completion and retrieve results
    // ... (see browser_demo.rs for full example)

    Ok(())
}
```

### Example 4: Run Complete Demo

```bash
# Demonstrates all 13 tools with real-world workflows
cargo run --example browser_demo
```

## Architecture

### Core Components

- **BrowserManager**: Global singleton managing browser lifecycle with thread-safe lazy initialization
- **Kromekover**: Stealth evasion system with 20+ JavaScript injection scripts
- **Agent System**: LLM-powered autonomous navigation using local inference (kodegen_candle_agent)
- **Research Sessions**: Background async research with session-based progress tracking
- **MCP Server**: HTTP/SSE transport integration managed by kodegend daemon

### Design Principles

1. **Memory Efficiency**: Single shared browser instance across all tools
2. **Async-First**: Built on tokio with proper async lock patterns
3. **Stealth**: Anti-detection via kromekover fingerprint masking
4. **Resilience**: Best-effort error handling with graceful degradation
5. **MCP Native**: First-class Model Context Protocol support

## Development

### Running Tests

```bash
# Run comprehensive tool demo (no unit tests currently)
cargo run --example browser_demo
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint with clippy
cargo clippy --all-targets --all-features

# Check without building
cargo check
```

### Adding a New Tool

1. Create tool implementation in `src/tools/your_tool.rs`
2. Implement `MCPTool` trait with schema and execute logic
3. Export from `src/tools/mod.rs`
4. Register in `src/main.rs` via `register_tool()`
5. Tool automatically appears in MCP tool listing

### Adding Kromekover Evasion Scripts

1. Add `.js` file to `src/kromekover/evasions/`
2. Add filename to `EVASION_SCRIPTS` array in `src/kromekover/mod.rs`
3. Ensure proper ordering (check dependencies)
4. Scripts auto-inject on browser launch

## Project Structure

```
kodegen-tools-browser/
├── src/
│   ├── agent/              # Autonomous agent system
│   ├── browser/            # Browser wrapper and lifecycle
│   ├── kromekover/         # Stealth evasion scripts
│   ├── page_extractor/     # Content extraction utilities
│   ├── research/           # Async research sessions
│   ├── tools/              # MCP tool implementations
│   ├── utils/              # Shared utilities
│   ├── web_search/         # Web search integration
│   ├── lib.rs              # Library exports
│   └── main.rs             # HTTP server binary
├── examples/
│   └── browser_demo.rs     # Comprehensive tool demonstration
├── config.yaml             # Browser configuration
└── Cargo.toml              # Project metadata
```

## Contributing

Contributions are welcome! Please ensure:

1. Code is formatted with `cargo fmt`
2. No clippy warnings: `cargo clippy --all-targets --all-features`
3. Existing examples still work: `cargo run --example browser_demo`

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Resources

- **Homepage**: https://kodegen.ai
- **Repository**: https://github.com/cyrup-ai/kodegen-tools-browser
- **MCP Protocol**: https://modelcontextprotocol.io
- **chromiumoxide**: https://github.com/mattsse/chromiumoxide

---

**Built by [KODEGEN.ᴀɪ](https://kodegen.ai)** - Memory-efficient, blazing-fast tools for code generation agents.
