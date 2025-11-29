// Category HTTP Server: Browser Tools
//
// This binary serves browser automation tools over HTTP/HTTPS transport.
// Managed by kodegend daemon, typically running on port 30440.

use anyhow::Result;
use kodegen_server_http::{run_http_server, Managers, RouterSet, ShutdownHook, register_tool, ConnectionCleanupFn};
use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;

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
        let browser_agent_tool = BrowserAgentTool::new(browser_manager.clone(), server_url.clone());
        let agent_registry = browser_agent_tool.get_registry().await;
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            browser_agent_tool,
        );

        // Long-running research tool (1 tool)
        let browser_research_tool = BrowserResearchTool::new(browser_manager.clone());
        let research_registry = browser_research_tool.get_registry().await;
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            browser_research_tool,
        );

        // Web search tool (1 tool)
        (tool_router, prompt_router) = register_tool(
            tool_router,
            prompt_router,
            BrowserWebSearchTool::new(),
        );

        // Create cleanup callback for connection dropped notification
        let cleanup: ConnectionCleanupFn = Arc::new(move |connection_id: String| {
            let agent_reg = agent_registry.clone();
            let research_reg = research_registry.clone();
            Box::pin(async move {
                let agent_cleaned = agent_reg.cleanup_connection(&connection_id).await;
                let research_cleaned = research_reg.cleanup_connection(&connection_id).await;
                log::info!(
                    "Connection {}: cleaned up {} browser agent(s), {} research session(s)",
                    connection_id,
                    agent_cleaned,
                    research_cleaned
                );
            }) as Pin<Box<dyn Future<Output = ()> + Send + 'static>>
        });

        let mut router_set = RouterSet::new(tool_router, prompt_router, managers);
        router_set.connection_cleanup = Some(cleanup);
        Ok(router_set)
        })
    })
    .await
}
