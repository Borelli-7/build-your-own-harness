//! Command-line interface definitions.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Provider {
    /// Offline stand-in that echoes input back; requires no API key.
    Echo,
    Openai,
    Anthropic,
}

#[derive(Debug, Parser)]
#[command(name = "build-your-own-harness", about = "A minimal coding-agent harness")]
pub struct Cli {
    /// LLM provider to use; can be switched at any time via this flag.
    #[arg(long, value_enum, default_value = "echo")]
    pub provider: Provider,

    /// Model name to request from the LLM provider.
    #[arg(long, default_value = "gpt-4o-mini")]
    pub model: String,

    /// API key for the selected provider; falls back to OPENAI_API_KEY/ANTHROPIC_API_KEY.
    #[arg(long)]
    pub api_key: Option<String>,

    /// Override the provider's API base URL (e.g. to target a self-hosted or compatible endpoint).
    #[arg(long)]
    pub base_url: Option<String>,

    /// Working directory that filesystem/process tools are sandboxed to.
    #[arg(long)]
    pub working_dir: Option<PathBuf>,

    /// Additional command names allowed for the `/run` tool, beyond the built-in safe defaults.
    #[arg(long = "allow-command", value_name = "COMMAND")]
    pub allow_commands: Vec<String>,
}

pub fn parse() -> Cli {
    Cli::parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_echo_provider() {
        let cli = Cli::try_parse_from(["harness"]).unwrap();
        assert_eq!(cli.provider, Provider::Echo);
        assert!(cli.allow_commands.is_empty());
    }

    #[test]
    fn parses_provider_and_extra_allowed_commands() {
        let cli = Cli::try_parse_from([
            "harness",
            "--provider",
            "anthropic",
            "--allow-command",
            "cargo",
            "--allow-command",
            "npm",
        ])
        .unwrap();
        assert_eq!(cli.provider, Provider::Anthropic);
        assert_eq!(
            cli.allow_commands,
            vec!["cargo".to_string(), "npm".to_string()]
        );
    }
}
