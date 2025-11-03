//! Browser automation tool implementations

mod browser_agent;
mod click;
mod extract_text;
mod get_research_result;
mod get_research_status;
mod list_research_sessions;
mod navigate;
mod screenshot;
mod scroll;
mod start_browser_research;
mod stop_browser_research;
mod type_text;
mod web_search;

pub use browser_agent::BrowserAgentTool;
pub use click::BrowserClickTool;
pub use extract_text::BrowserExtractTextTool;
pub use get_research_result::GetResearchResultTool;
pub use get_research_status::GetResearchStatusTool;
pub use list_research_sessions::ListResearchSessionsTool;
pub use navigate::BrowserNavigateTool;
pub use screenshot::BrowserScreenshotTool;
pub use scroll::BrowserScrollTool;
pub use start_browser_research::StartBrowserResearchTool;
pub use stop_browser_research::StopBrowserResearchTool;
pub use type_text::BrowserTypeTextTool;
pub use web_search::WebSearchTool;
