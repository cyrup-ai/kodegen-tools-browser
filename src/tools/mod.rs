//! Browser automation tool implementations

mod browser_agent;
mod browser_research;
mod browser_web_search;
mod click;
mod extract_text;
mod navigate;
mod screenshot;
mod scroll;
mod type_text;

pub use browser_agent::BrowserAgentTool;
pub use browser_research::BrowserResearchTool;
pub use browser_web_search::BrowserWebSearchTool;
pub use click::BrowserClickTool;
pub use extract_text::BrowserExtractTextTool;
pub use navigate::BrowserNavigateTool;
pub use screenshot::BrowserScreenshotTool;
pub use scroll::BrowserScrollTool;
pub use type_text::BrowserTypeTextTool;
