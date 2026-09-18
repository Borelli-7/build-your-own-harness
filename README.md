# Build Your Own Harness

A minimal terminal-based coding-agent harness written in Rust: a TUI chat loop backed by a
pluggable LLM client and a small set of sandboxed tools (filesystem + shell commands).

## Architecture

```
src/
├── main.rs          # entry point: tracing init, CLI parsing, hands off to app::run
├── cli.rs            # clap-based CLI definitions (provider, model, api key, sandbox root, allowlist)
├── app.rs             # top-level orchestration: owns App state and the main event loop
├── state.rs           # AppState enum + transition(event) -> AppState (the state machine)
├── events.rs          # Event enum consumed by the state machine
├── llm/
│   ├── mod.rs          # re-exports
│   └── client.rs       # LlmClient trait + EchoClient, OpenAiClient, AnthropicClient
├── tools/
│   ├── mod.rs
│   ├── filesystem.rs   # sandboxed read/write/list, rejects '..' and absolute paths
│   └── process.rs      # CommandAllowlist + sandboxed shell command execution
└── ui/
    ├── mod.rs
    └── terminal.rs      # ratatui/crossterm render loop, input polling, terminal guard
```

This mirrors a typical agent-harness layering:

1. **CLI** (`cli.rs`) parses flags and env vars into a `Cli` struct.
2. **App** (`app.rs`) builds the LLM client and tool allowlist from the `Cli`, then owns the
   single async event loop for the process.
3. **State machine** (`state.rs` + `events.rs`) tracks what the app is doing (`Idle`,
   `AwaitingLlm`, `ExecutingTool`, `Error`, `Quitting`) and transitions purely as a function of
   incoming `Event`s — no I/O happens inside `transition()`.
4. **LLM module** (`llm/`) abstracts the model provider behind a single `LlmClient` trait so the
   app loop never needs to know which provider is in use.
5. **Tools** (`tools/`) implement the actual side effects (file I/O, process execution), each
   sandboxed independently of the state machine and LLM client.
6. **UI** (`ui/terminal.rs`) is the only module touching the terminal: it renders the
   conversation history + input box and turns key events into an `InputEvent`.

## How it works

`app::run` builds an `App` (LLM client, working directory, command allowlist) and enters
`App::event_loop`, which on every tick:

1. Renders the current history + input buffer via `ui::terminal::render`.
2. Polls for a key event (100ms timeout) and turns it into an `InputEvent`
   (`Char`, `Backspace`, `Submit`, `Quit`, `Tick`).
3. On `Submit`, dispatches the typed line in `App::handle_input`:
   - `/read <path>`, `/write <path> <contents>`, `/ls <path>` → `tools::filesystem`
   - `/run <command> [args...]` → `tools::process::run_command`, checked against the
     `CommandAllowlist` first
   - anything else → sent to the configured `LlmClient::stream_chat`, streamed chunk-by-chunk
     into the transcript
4. Every dispatch pushes an `Event` through `AppState::transition` so the header always reflects
   what the harness is currently doing.

A `TerminalGuard` (`Drop` impl) restores raw mode / the normal screen buffer even if the process
panics, so a crash never leaves your shell in a broken state.

### LLM providers

`llm::client` defines one trait:

```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn stream_chat(&self, messages: &[Message]) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>>;
}
```

Three implementations ship today, selected at runtime via `--provider`:

| Provider     | Notes                                                                 |
|--------------|------------------------------------------------------------------------|
| `echo`       | Offline default. Echoes the last message back word by word. No key needed. |
| `openai`     | Streams from any OpenAI-compatible `/v1/chat/completions` endpoint (SSE). |
| `anthropic`  | Streams from Anthropic's `/v1/messages` endpoint (SSE `content_block_delta`). |

`--base-url` lets you point `openai` at any compatible endpoint (Ollama, OpenRouter, vLLM, etc.).

### Tool sandboxing

- **Filesystem** (`tools/filesystem.rs`): every path is joined onto the configured working
  directory; absolute paths and any `..` component are rejected outright.
- **Process** (`tools/process.rs`): `/run` only executes commands present in a
  `CommandAllowlist`. The default allowlist is intentionally small and read-only-ish
  (`ls, cat, pwd, echo, git, grep, find, wc`); extend it per-run with `--allow-command <name>`.
  Commands are spawned directly via `tokio::process::Command` (no shell), so there's no shell
  injection risk from argument content.

## Running it

```bash
cargo build
cargo run -- --help
```

### CLI options

```
--provider <echo|openai|anthropic>   LLM provider to use [default: echo]
--model <MODEL>                      Model name to request [default: gpt-4o-mini]
--api-key <KEY>                      API key; falls back to OPENAI_API_KEY / ANTHROPIC_API_KEY
--base-url <URL>                     Override the provider's API base URL
--working-dir <DIR>                  Sandbox root for filesystem/process tools [default: cwd]
--allow-command <COMMAND>            Add a command to the allowlist (repeatable)
```

### Examples

```bash
# Offline smoke test, no API key required
cargo run

# OpenAI, reading the key from the environment
OPENAI_API_KEY=sk-... cargo run -- --provider openai --model gpt-4o-mini

# Anthropic, explicit key and a custom sandbox root
cargo run -- --provider anthropic --api-key sk-ant-... --working-dir ./workspace

# Allow the agent to also run `cargo` commands via /run
cargo run -- --allow-command cargo
```

Inside the TUI:

- Type a message and press `Enter` to send it to the LLM.
- `/read <path>`, `/write <path> <contents>`, `/ls <path>` to exercise the filesystem tool.
- `/run <command> [args...]` to exercise the process tool (must be allowlisted).
- `Esc` or `Ctrl+C` to quit.

## Testing

```bash
cargo test           # unit tests for state transitions, CLI parsing, tools, LLM clients
cargo clippy --all-targets
```
