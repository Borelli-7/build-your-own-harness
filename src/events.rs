//! App-level events consumed by the state machine and the orchestration loop.

#[derive(Debug, Clone)]
pub enum Event {
    UserInput(String),
    LlmChunk(String),
    LlmDone,
    ToolCall(String),
    ToolResult(String),
    Error(String),
    Quit,
}
