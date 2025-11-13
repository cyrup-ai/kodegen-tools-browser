//! Browser automation tool implementations

mod browser_agent;
mod browser_get_research_result;
mod browser_get_research_status;
mod browser_list_research_sessions;
mod browser_start_research;
mod browser_stop_research;
mod browser_web_search;
mod click;
mod extract_text;
mod navigate;
mod screenshot;
mod scroll;
mod type_text;

pub use browser_agent::BrowserAgentTool;
pub use browser_get_research_result::BrowserGetResearchResultTool;
pub use browser_get_research_status::BrowserGetResearchStatusTool;
pub use browser_list_research_sessions::BrowserListResearchSessionsTool;
pub use browser_start_research::BrowserStartResearchTool;
pub use browser_stop_research::BrowserStopResearchTool;
pub use browser_web_search::BrowserWebSearchTool;
pub use click::BrowserClickTool;
pub use extract_text::BrowserExtractTextTool;
pub use navigate::BrowserNavigateTool;
pub use screenshot::BrowserScreenshotTool;
pub use scroll::BrowserScrollTool;
pub use type_text::BrowserTypeTextTool;
