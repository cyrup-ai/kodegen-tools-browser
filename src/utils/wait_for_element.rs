//! Element polling utility for SPA support
//!
//! Provides wait_for_element() which polls for DOM elements with exponential backoff.
//! This is critical for Single Page Applications (SPAs) that render elements via JavaScript
//! after the initial page load event fires.

use std::time::Duration;

use chromiumoxide::Page;
use chromiumoxide::element::Element;
use kodegen_mcp_schema::McpError;

/// Wait for an element to appear in the DOM using exponential backoff polling
///
/// This function polls for an element with exponential backoff, waiting for SPAs
/// to render elements after page load. Used by navigate, click, and type_text tools.
///
/// # Arguments
/// * `page` - The chromiumoxide Page to search in
/// * `selector` - CSS selector for the element
/// * `timeout` - Maximum time to wait for the element
///
/// # Returns
/// * `Ok(Element)` - The element was found
/// * `Err(McpError)` - Timeout exceeded or other error
///
/// # Polling Strategy
/// - Starts at 100ms intervals
/// - Doubles each retry (exponential backoff)
/// - Caps at 1 second maximum interval
/// - Total duration limited by timeout parameter
pub async fn wait_for_element(
    page: &Page,
    selector: &str,
    timeout: Duration,
) -> Result<Element, McpError> {
    let start = std::time::Instant::now();
    let mut poll_interval = Duration::from_millis(100); // Start with 100ms
    let max_interval = Duration::from_secs(1); // Cap at 1 second

    loop {
        // Try to find element
        if let Ok(element) = page.find_element(selector).await {
            return Ok(element);
        }

        // Check timeout
        if start.elapsed() >= timeout {
            return Err(McpError::Other(anyhow::anyhow!(
                "Element not found (timeout after {}ms): '{}'. \
                 Try: (1) Verify selector is correct using browser dev tools, \
                 (2) Ensure element is visible and loaded, \
                 (3) Increase timeout_ms parameter.",
                timeout.as_millis(),
                selector
            )));
        }

        // Wait with exponential backoff
        tokio::time::sleep(poll_interval).await;

        // Double the interval, but cap at max_interval
        poll_interval = (poll_interval * 2).min(max_interval);
    }
}
