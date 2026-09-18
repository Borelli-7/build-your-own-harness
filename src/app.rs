//! Top-level orchestration: owns app state and wires the UI, LLM client, and tools together.

use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::cli::{Cli, Provider};
use crate::events::Event;
use crate::llm::{AnthropicClient, EchoClient, LlmClient, Message, OpenAiClient};
use crate::state::AppState;
use crate::tools::process::CommandAllowlist;
use crate::tools::{filesystem, process};
use crate::ui::terminal::{self, InputEvent};

pub struct App {
    state: AppState,
    history: Vec<(String, String)>,
    input: String,
    llm: Box<dyn LlmClient>,
    working_dir: PathBuf,
    allowlist: CommandAllowlist,
}

/// Resolves the API key from `--api-key`, falling back to the provider's conventional env var.
fn resolve_api_key(cli: &Cli) -> Option<String> {
    resolve_api_key_with(cli, |name| std::env::var(name).ok())
}

/// Testable version of [`resolve_api_key`] that takes an injectable env lookup.
fn resolve_api_key_with(cli: &Cli, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(key) = &cli.api_key {
        return Some(key.clone());
    }
    match cli.provider {
        Provider::Openai => lookup("OPENAI_API_KEY"),
        Provider::Anthropic => lookup("ANTHROPIC_API_KEY"),
        Provider::Echo => None,
    }
}

/// Maps internal history roles onto the "user"/"assistant" roles providers understand.
fn normalize_role(role: &str) -> &'static str {
    match role {
        "assistant" => "assistant",
        _ => "user",
    }
}

fn build_llm(cli: &Cli) -> anyhow::Result<Box<dyn LlmClient>> {
    Ok(match cli.provider {
        Provider::Echo => Box::new(EchoClient),
        Provider::Openai => {
            let key = resolve_api_key(cli)
                .context("--api-key or OPENAI_API_KEY is required for provider openai")?;
            Box::new(OpenAiClient::new(key, cli.model.clone(), cli.base_url.clone()))
        }
        Provider::Anthropic => {
            let key = resolve_api_key(cli)
                .context("--api-key or ANTHROPIC_API_KEY is required for provider anthropic")?;
            Box::new(AnthropicClient::new(key, cli.model.clone(), cli.base_url.clone()))
        }
    })
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let llm = build_llm(&cli)?;
    let working_dir = match cli.working_dir.clone() {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let allowlist = CommandAllowlist::new(cli.allow_commands.clone());

    let mut app = App {
        state: AppState::Idle,
        history: Vec::new(),
        input: String::new(),
        llm,
        working_dir,
        allowlist,
    };

    let mut term = terminal::setup()?;
    let _guard = terminal::TerminalGuard;
    let result = app.event_loop(&mut term).await;
    terminal::restore(&mut term)?;
    result
}

impl App {
    async fn event_loop(&mut self, term: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
        loop {
            term.draw(|f| terminal::render(f, &self.history, &self.input, &self.state))?;

            match terminal::poll_input(Duration::from_millis(100))? {
                InputEvent::Char(c) => self.input.push(c),
                InputEvent::Backspace => {
                    self.input.pop();
                }
                InputEvent::Submit => {
                    let text = std::mem::take(&mut self.input);
                    if !text.is_empty() {
                        self.handle_input(text).await?;
                    }
                }
                InputEvent::Quit => {
                    self.state = self.state.transition(&Event::Quit);
                }
                InputEvent::Tick => {}
            }

            if self.state == AppState::Quitting {
                break;
            }
        }
        Ok(())
    }

    async fn handle_input(&mut self, text: String) -> anyhow::Result<()> {
        self.history.push(("you".into(), text.clone()));

        if let Some(path) = text.strip_prefix("/read ") {
            self.state = self.state.transition(&Event::ToolCall(text.clone()));
            let result = filesystem::read_file(&self.working_dir, path.trim()).await;
            self.finish_tool(result);
        } else if let Some(rest) = text.strip_prefix("/write ") {
            self.state = self.state.transition(&Event::ToolCall(text.clone()));
            let (path, contents) = rest.split_once(' ').unwrap_or((rest, ""));
            let result = filesystem::write_file(&self.working_dir, path.trim(), contents)
                .await
                .map(|_| format!("wrote {path}"));
            self.finish_tool(result);
        } else if let Some(path) = text.strip_prefix("/ls ") {
            self.state = self.state.transition(&Event::ToolCall(text.clone()));
            let result = filesystem::list_dir(&self.working_dir, path.trim())
                .await
                .map(|entries| entries.join("\n"));
            self.finish_tool(result);
        } else if let Some(rest) = text.strip_prefix("/run ") {
            self.state = self.state.transition(&Event::ToolCall(text.clone()));
            let mut parts = rest.split_whitespace();
            let cmd = parts.next().unwrap_or_default().to_string();
            let args: Vec<String> = parts.map(String::from).collect();
            let result = process::run_command(&self.working_dir, &self.allowlist, &cmd, &args)
                .await
                .map(|out| format!("[exit {}]\n{}{}", out.status, out.stdout, out.stderr));
            self.finish_tool(result);
        } else {
            self.state = self.state.transition(&Event::UserInput(text));
            self.ask_llm().await?;
        }
        Ok(())
    }

    fn finish_tool(&mut self, result: anyhow::Result<String>) {
        let output = match result {
            Ok(output) => {
                self.history.push(("tool".into(), output.clone()));
                output
            }
            Err(err) => {
                let msg = err.to_string();
                self.history.push(("tool-error".into(), msg.clone()));
                msg
            }
        };
        self.state = self.state.transition(&Event::ToolResult(output));
    }

    async fn ask_llm(&mut self) -> anyhow::Result<()> {
        let messages: Vec<Message> = self
            .history
            .iter()
            .map(|(role, content)| Message {
                role: normalize_role(role).to_string(),
                content: content.clone(),
            })
            .collect();

        let mut stream = self.llm.stream_chat(&messages).await?;
        let mut reply = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(text) => {
                    reply.push_str(&text);
                    self.state = self.state.transition(&Event::LlmChunk(text));
                }
                Err(err) => {
                    self.history.push(("error".into(), err.to_string()));
                    self.state = self.state.transition(&Event::Error(err.to_string()));
                    return Ok(());
                }
            }
        }
        self.history.push(("assistant".into(), reply));
        self.state = self.state.transition(&Event::LlmDone);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cli_with(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("harness").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn normalize_role_maps_internal_roles_to_user_or_assistant() {
        assert_eq!(normalize_role("assistant"), "assistant");
        assert_eq!(normalize_role("you"), "user");
        assert_eq!(normalize_role("tool"), "user");
        assert_eq!(normalize_role("tool-error"), "user");
    }

    #[test]
    fn resolve_api_key_prefers_explicit_flag_over_env() {
        let cli = cli_with(&["--provider", "openai", "--api-key", "sk-test"]);
        let key = resolve_api_key_with(&cli, |_| Some("from-env".to_string()));
        assert_eq!(key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn resolve_api_key_falls_back_to_provider_env_var() {
        let cli = cli_with(&["--provider", "anthropic"]);
        let key =
            resolve_api_key_with(&cli, |name| (name == "ANTHROPIC_API_KEY").then(|| "from-env".to_string()));
        assert_eq!(key.as_deref(), Some("from-env"));
    }

    #[test]
    fn resolve_api_key_is_none_for_echo_provider() {
        let cli = cli_with(&["--provider", "echo"]);
        let key = resolve_api_key_with(&cli, |_| Some("should-not-be-used".to_string()));
        assert_eq!(key, None);
    }

    #[test]
    fn build_llm_succeeds_for_echo_without_any_key() {
        let cli = cli_with(&["--provider", "echo"]);
        assert!(build_llm(&cli).is_ok());
    }

    #[test]
    fn build_llm_succeeds_when_api_key_flag_is_set() {
        let cli = cli_with(&["--provider", "openai", "--api-key", "sk-test"]);
        assert!(build_llm(&cli).is_ok());
    }
}
