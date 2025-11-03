//! Page metadata extraction module
//!
//! This module provides efficient page metadata extraction using JavaScript evaluation.
//! Ported from citescrape's production-tested page_extractor.

pub mod schema;
pub mod js_scripts;
pub mod extractors;
pub mod page_info;

// Re-export commonly used types
pub use schema::PageMetadata;
pub use page_info::{PageInfo, extract_page_info};
