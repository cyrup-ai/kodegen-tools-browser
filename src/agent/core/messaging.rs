use crate::agent::AgentOutput;

/// Agent command enum for internal message passing
pub(super) enum AgentCommand {
    RunStep,
    Stop,
}

/// Agent response enum for internal message passing
#[derive(Debug)]
pub(super) enum AgentResponse {
    StepComplete(AgentOutput),
    Error(String),
    Stopped,
}
