//! Research module for async browser research operations

pub mod session_manager;

pub use session_manager::{
    ResearchSession, ResearchSessionManager, ResearchStatus, ResearchStep,
};
