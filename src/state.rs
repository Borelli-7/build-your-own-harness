//! Application state and the transitions driven by incoming `Event`s.

use crate::events::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppState {
    Idle,
    AwaitingLlm,
    ExecutingTool,
    Error(String),
    Quitting,
}

impl AppState {
    pub fn transition(&self, event: &Event) -> AppState {
        match event {
            Event::Quit => AppState::Quitting,
            Event::Error(msg) => AppState::Error(msg.clone()),
            Event::UserInput(text) => {
                tracing::debug!(%text, "received user input");
                AppState::AwaitingLlm
            }
            Event::LlmChunk(text) => {
                tracing::trace!(%text, "received llm chunk");
                AppState::AwaitingLlm
            }
            Event::LlmDone => AppState::Idle,
            Event::ToolCall(command) => {
                tracing::debug!(%command, "starting tool call");
                AppState::ExecutingTool
            }
            Event::ToolResult(output) => {
                tracing::debug!(%output, "tool call finished");
                AppState::Idle
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_input_moves_idle_to_awaiting_llm() {
        let state = AppState::Idle.transition(&Event::UserInput("hi".into()));
        assert_eq!(state, AppState::AwaitingLlm);
    }

    #[test]
    fn llm_done_returns_to_idle() {
        let state = AppState::AwaitingLlm.transition(&Event::LlmDone);
        assert_eq!(state, AppState::Idle);
    }

    #[test]
    fn tool_call_then_result_round_trips_through_executing_tool() {
        let state = AppState::Idle.transition(&Event::ToolCall("/run ls".into()));
        assert_eq!(state, AppState::ExecutingTool);
        let state = state.transition(&Event::ToolResult("output".into()));
        assert_eq!(state, AppState::Idle);
    }

    #[test]
    fn error_event_always_wins() {
        let state = AppState::AwaitingLlm.transition(&Event::Error("boom".into()));
        assert_eq!(state, AppState::Error("boom".into()));
    }

    #[test]
    fn quit_event_always_wins() {
        let state = AppState::ExecutingTool.transition(&Event::Quit);
        assert_eq!(state, AppState::Quitting);
    }
}
