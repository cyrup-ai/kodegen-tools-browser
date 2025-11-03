// All browser utility modules - no feature gating
mod agent_state;
pub mod constants;
mod deep_research;
mod errors;
mod timeout;
mod wait_for_element;

pub use agent_state::AgentState;
pub use deep_research::{DeepResearch, ResearchOptions, ResearchResult};
pub use timeout::{validate_interaction_timeout, validate_navigation_timeout};
pub use wait_for_element::wait_for_element;

// /// Result type for utility functions
// pub type UtilsResult<T> = Result<T, UtilsError>;
