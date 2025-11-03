//! Simplified page info extraction for deep research
//!
//! This module provides a lightweight wrapper around citescrape's extractors
//! for use in deep_research, without link rewriting or content saving dependencies.

use anyhow::{Context, Result};
use chromiumoxide::Page;

use super::extractors::extract_metadata;
use super::schema::PageMetadata;

/// Lightweight page information for research results
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageInfo {
    /// Page title from document.title
    pub title: String,
    /// Page metadata from meta tags and OpenGraph
    pub metadata: PageMetadata,
}

/// Extract page title and metadata in parallel
///
/// Uses the same parallel extraction pattern as citescrape's extract_page_data
/// but without link rewriting or content saving dependencies.
///
/// # Performance
/// Runs JavaScript evaluations in parallel via tokio::try_join! for ~2x speedup
/// compared to sequential extraction.
///
/// # Errors
/// Returns error if JavaScript evaluation fails or page is not loaded.
///
/// # Example
/// ```rust
/// let page_info = extract_page_info(page.clone()).await?;
/// println!("Title: {}", page_info.title);
/// println!("Description: {:?}", page_info.metadata.description);
/// ```
pub async fn extract_page_info(page: Page) -> Result<PageInfo> {
    // Launch title and metadata extraction in parallel (2x speedup)
    let (title, metadata) = tokio::try_join!(
        // Title extraction (inline, no separate script needed)
        async {
            let title_value = page
                .evaluate("document.title")
                .await
                .context("Failed to evaluate document.title")?
                .into_value()
                .map_err(|e| anyhow::anyhow!("Failed to get page title: {e}"))?;

            if let serde_json::Value::String(title) = title_value {
                Ok(title)
            } else {
                Ok(String::new())
            }
        },
        // Metadata extraction (uses citescrape's proven extractor)
        extract_metadata(page.clone()),
    )?;

    Ok(PageInfo { title, metadata })
}
