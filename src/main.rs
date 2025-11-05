// Category HTTP Server: Browser Tools
//
// This binary serves browser automation tools over HTTP/HTTPS transport.
// Managed by kodegend daemon, typically running on port 30440.

use anyhow::Result;
use kodegen_server_http::{run_http_server, Managers, RouterSet, ShutdownHook, register_tool};
use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};
use std::sync::Arc;

// Wrapper to impl ShutdownHook for Arc<BrowserManager>
struct BrowserManagerWrapper(Arc<kodegen_tools_browser::BrowserManager>);

impl ShutdownHook for BrowserManagerWrapper {
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let manager = self.0.clone();
        Box::pin(async move {
            manager.shutdown().await
        })
    }
}

// Wrapper to impl ShutdownHook for ResearchSessionManager singleton
struct ResearchSessionManagerWrapper;

impl ShutdownHook for ResearchSessionManagerWrapper {
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            kodegen_tools_browser::research::ResearchSessionManager::global().shutdown().await
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    run_http_server("browser", |_config, _tracker| {
        Box::pin(async move {
        let mut tool_router = ToolRouter::new();
        let mut prompt_router = PromptRouter::new();
        let managers = Managers::new();

        // Fixed server URL for browser loopback tools (port 30438 managed by daemon)
        let server_url = "http://127.0.0.1:30438/mcp".to_string();

        // Initialize browser manager (global singleton)
        let browser_manager = kodegen_tools_browser::BrowserManager::global();
        managers.register(BrowserManagerWrapper(browser_manager.clone())).await;

        // Register research session manager for graceful cleanup task shutdown
        managers.register(ResearchSessionManagerWrapper).await;

        // Register all browser tools
        use kodegen_tools_browser::*;

        // Core browser automation tools (6 tools)
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserNavigateTool::new(browser_manager.clone()),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserClickTool::new(browser_manager.clone()),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserTypeTextTool::new(browser_manager.clone()),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserScreenshotTool::new(browser_manager.clone()),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserExtractTextTool::new(browser_manager.clone()),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserScrollTool::new(browser_manager.clone()),
        );

        // Advanced browser tools (1 tool)
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserAgentTool::new(browser_manager.clone(), server_url.clone()),
        );

        // Async research session tools (5 tools)
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            StartBrowserResearchTool::new(),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            GetResearchStatusTool::new(),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            GetResearchResultTool::new(),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            StopBrowserResearchTool::new(),
        );
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            ListResearchSessionsTool::new(),
        );

        // Web search tool (1 tool)
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            WebSearchTool::new(),
        );

        Ok(RouterSet::new(tool_router, prompt_router, managers))
        })
    })
    .await
}
