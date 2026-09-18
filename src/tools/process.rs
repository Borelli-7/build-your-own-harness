//! Process-execution tool for running shell commands scoped to a working directory.

use std::collections::HashSet;
use std::path::Path;

use anyhow::bail;
use tokio::process::Command;

#[derive(Debug)]
pub struct ProcessOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Read-only-ish commands considered safe to run without extra opt-in.
const DEFAULT_ALLOWED_COMMANDS: &[&str] = &["ls", "cat", "pwd", "echo", "git", "grep", "find", "wc"];

/// Set of command names permitted for the `/run` tool; nothing outside this set is executed.
#[derive(Debug, Clone)]
pub struct CommandAllowlist {
    allowed: HashSet<String>,
}

impl CommandAllowlist {
    pub fn new(extra: impl IntoIterator<Item = String>) -> Self {
        let mut allowed: HashSet<String> = DEFAULT_ALLOWED_COMMANDS.iter().map(|s| s.to_string()).collect();
        allowed.extend(extra);
        Self { allowed }
    }

    pub fn is_allowed(&self, command: &str) -> bool {
        self.allowed.contains(command)
    }
}

impl Default for CommandAllowlist {
    fn default() -> Self {
        Self::new(std::iter::empty())
    }
}

/// Runs `command` inside `cwd` if it is present in `allowlist`; rejected otherwise.
pub async fn run_command(
    cwd: &Path,
    allowlist: &CommandAllowlist,
    command: &str,
    args: &[String],
) -> anyhow::Result<ProcessOutput> {
    if !allowlist.is_allowed(command) {
        bail!("command '{command}' is not in the allowlist");
    }
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .output()
        .await?;
    Ok(ProcessOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_allowlist_permits_known_commands() {
        let allowlist = CommandAllowlist::default();
        assert!(allowlist.is_allowed("ls"));
        assert!(allowlist.is_allowed("git"));
    }

    #[test]
    fn default_allowlist_denies_unknown_commands() {
        let allowlist = CommandAllowlist::default();
        assert!(!allowlist.is_allowed("rm"));
        assert!(!allowlist.is_allowed("curl"));
    }

    #[test]
    fn extra_commands_can_be_opted_in() {
        let allowlist = CommandAllowlist::new(["cargo".to_string()]);
        assert!(allowlist.is_allowed("cargo"));
    }

    #[tokio::test]
    async fn run_command_rejects_disallowed_commands() {
        let dir = tempfile::tempdir().unwrap();
        let allowlist = CommandAllowlist::default();
        let err = run_command(dir.path(), &allowlist, "rm", &["-rf".to_string(), "/".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not in the allowlist"));
    }

    #[tokio::test]
    async fn run_command_executes_allowed_commands() {
        let dir = tempfile::tempdir().unwrap();
        let allowlist = CommandAllowlist::default();
        let output = run_command(dir.path(), &allowlist, "echo", &["hi".to_string()])
            .await
            .unwrap();
        assert_eq!(output.stdout.trim(), "hi");
        assert_eq!(output.status, 0);
    }
}
