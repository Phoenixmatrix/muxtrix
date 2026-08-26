//! Cross-platform process launch planning and PTY ownership.

use std::io::{Read, Write};
use std::path::PathBuf;

use muxtrix_domain::{LaunchProfile, ProcessBackend};
use portable_pty::{Child, CommandBuilder, MasterPty, native_pty_system};
use thiserror::Error;

pub use portable_pty::PtySize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(String, String)>,
}

impl LaunchPlan {
    pub fn from_profile(profile: &LaunchProfile) -> Result<Self, PlatformError> {
        if profile.program.trim().is_empty() && matches!(profile.backend, ProcessBackend::Local) {
            return Err(PlatformError::EmptyProgram);
        }

        match &profile.backend {
            ProcessBackend::Local => Ok(Self {
                executable: profile.program.clone(),
                arguments: profile.arguments.clone(),
                working_directory: profile.working_directory.clone(),
                environment: vec![("TERM".into(), "xterm-256color".into())],
            }),
            ProcessBackend::Wsl { distribution } => {
                let mut arguments = Vec::new();
                if let Some(distribution) = distribution {
                    if distribution.trim().is_empty() {
                        return Err(PlatformError::EmptyWslDistribution);
                    }
                    arguments.extend(["--distribution".into(), distribution.clone()]);
                }
                if let Some(working_directory) = &profile.working_directory {
                    let directory = working_directory
                        .to_str()
                        .ok_or_else(|| PlatformError::NonUtf8WslPath(working_directory.clone()))?;
                    arguments.extend(["--cd".into(), directory.into()]);
                }
                // Explicit login-shell mode keeps ConPTY launches aligned with
                // an interactive terminal: WSL resolves the selected user's
                // shell from the distribution and loads its login environment.
                // Without it, embedded hosts can fall back to a standard Bash
                // session even when the account is configured for zsh or fish.
                if profile.program.trim().is_empty() {
                    arguments.extend(["--shell-type".into(), "login".into()]);
                } else {
                    arguments.push("--exec".into());
                    arguments.push(profile.program.clone());
                    arguments.extend(profile.arguments.iter().cloned());
                }

                Ok(Self {
                    executable: "wsl.exe".into(),
                    arguments,
                    working_directory: None,
                    environment: Vec::new(),
                })
            }
        }
    }

    fn command_builder(&self) -> CommandBuilder {
        let mut command = CommandBuilder::new(&self.executable);
        command.args(&self.arguments);
        if let Some(working_directory) = &self.working_directory {
            command.cwd(working_directory);
        }
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        command
    }
}

/// OSC 7 working-directory reporting for shells that do not emit it on
/// their own. Fish reports out of the box; bash and zsh need a hook, and
/// these are the smallest ones that survive a trip through `wsl.exe`.
pub mod shell_integration {
    /// bash runs `PROMPT_COMMAND` before every prompt and, crucially, reads
    /// it from the environment — no rc-file edits or `--init-file` games.
    /// A user rc that appends to it composes; one that overwrites wins.
    /// Keep this free of `${...}`: editors such as Zed re-spawn the shell via
    /// `fish -i -c "exec env 'PROMPT_COMMAND=...' ..."` and fish rejects `${`
    /// inside double quotes, which killed the whole launch.
    pub const BASH_PROMPT_COMMAND: &str =
        r#"printf '\033]7;file://%s%s\033\\' "$HOSTNAME" "$PWD""#;

    /// zsh has no environment-borne hook, but it does read
    /// `$ZDOTDIR/.zshenv` first. Pointing ZDOTDIR at a directory holding
    /// this file installs the precmd hook, restores the user's real
    /// ZDOTDIR, and chain-sources their own .zshenv so nothing is lost.
    /// fish only volunteers OSC 7 to an allowlist of terminals (foot,
    /// kitty, VTE, Apple Terminal, WezTerm, iTerm) — under a plain
    /// xterm-256color it stays silent. This conf.d snippet reports
    /// unconditionally, but only inside Muxtrix panes (MUXTRIX_PANE_ID)
    /// so the file is inert for every other fish session.
    pub const FISH_CONF_D: &str = r#"# Muxtrix shell integration: report the working directory via OSC 7.
# Inert outside Muxtrix panes; safe to delete — Muxtrix recreates it.
if status is-interactive; and set -q MUXTRIX_PANE_ID
    function __muxtrix_report_pwd --on-variable PWD --description 'Report $PWD to Muxtrix via OSC 7'
        if status is-command-substitution; or set -q INSIDE_EMACS
            return
        end
        printf '\e]7;file://%s%s\e\\' $hostname (string escape --style=url -- $PWD)
    end
    __muxtrix_report_pwd
end
"#;

    pub const ZSH_ZSHENV: &str = r#"# Muxtrix shell integration: report the working directory via OSC 7.
if [ -n "${MUXTRIX_ORIG_ZDOTDIR-}" ]; then
    ZDOTDIR="$MUXTRIX_ORIG_ZDOTDIR"
    unset MUXTRIX_ORIG_ZDOTDIR
else
    unset ZDOTDIR
fi
if [ -f "${ZDOTDIR:-$HOME}/.zshenv" ]; then
    . "${ZDOTDIR:-$HOME}/.zshenv"
fi
_muxtrix_report_pwd() {
    printf '\033]7;file://%s%s\033\\' "${HOST:-}" "$PWD"
}
if [[ ${precmd_functions[(Ie)_muxtrix_report_pwd]} -eq 0 ]]; then
    precmd_functions+=(_muxtrix_report_pwd)
fi
"#;
}

/// A running child attached to a native PTY or ConPTY.
///
/// The reader and writer are separate so the session actor can dedicate one
/// blocking thread to PTY output while retaining input and resize control.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
}

impl PtySession {
    pub fn spawn(plan: &LaunchPlan, size: PtySize) -> Result<Self, PlatformError> {
        let system = native_pty_system();
        let pair = system
            .openpty(size)
            .map_err(|error| PlatformError::Pty(error.to_string()))?;
        let child = pair
            .slave
            .spawn_command(plan.command_builder())
            .map_err(|error| PlatformError::Pty(error.to_string()))?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| PlatformError::Pty(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| PlatformError::Pty(error.to_string()))?;

        Ok(Self {
            master: pair.master,
            child,
            reader: Some(reader),
            writer,
        })
    }

    pub fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, PlatformError> {
        self.reader.take().ok_or(PlatformError::ReaderAlreadyTaken)
    }

    /// The operating-system process id of the spawned child, when known.
    #[must_use]
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), PlatformError> {
        self.writer
            .write_all(bytes)
            .map_err(|error| PlatformError::Io(error.to_string()))?;
        self.writer
            .flush()
            .map_err(|error| PlatformError::Io(error.to_string()))
    }

    pub fn resize(&self, size: PtySize) -> Result<(), PlatformError> {
        self.master
            .resize(size)
            .map_err(|error| PlatformError::Pty(error.to_string()))
    }

    pub fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>, PlatformError> {
        self.child
            .try_wait()
            .map_err(|error| PlatformError::Pty(error.to_string()))
    }

    pub fn kill(&mut self) -> Result<(), PlatformError> {
        self.child
            .kill()
            .map_err(|error| PlatformError::Pty(error.to_string()))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlatformError {
    #[error("launch profile program cannot be empty")]
    EmptyProgram,
    #[error("WSL distribution cannot be empty")]
    EmptyWslDistribution,
    #[error("WSL working directory is not valid UTF-8: {0:?}")]
    NonUtf8WslPath(PathBuf),
    #[error("PTY operation failed: {0}")]
    Pty(String),
    #[error("I/O operation failed: {0}")]
    Io(String),
    #[error("PTY reader has already been taken")]
    ReaderAlreadyTaken,
}

#[cfg(test)]
mod tests {
    use muxtrix_domain::ProfileId;

    use super::*;

    #[test]
    fn local_profile_stays_on_the_host() -> Result<(), PlatformError> {
        let profile = LaunchProfile {
            id: ProfileId::new(),
            name: "Local shell".into(),
            backend: ProcessBackend::Local,
            program: "/bin/bash".into(),
            arguments: vec!["-l".into()],
            working_directory: Some(PathBuf::from("/work")),
        };

        let plan = LaunchPlan::from_profile(&profile)?;
        assert_eq!(plan.executable, "/bin/bash");
        assert_eq!(plan.arguments, ["-l"]);
        assert_eq!(plan.working_directory, Some(PathBuf::from("/work")));
        assert_eq!(plan.environment, [("TERM".into(), "xterm-256color".into())]);
        Ok(())
    }

    #[test]
    fn wsl_profile_is_explicit_and_does_not_leak_a_windows_cwd() -> Result<(), PlatformError> {
        let profile = LaunchProfile {
            id: ProfileId::new(),
            name: "Ubuntu".into(),
            backend: ProcessBackend::Wsl {
                distribution: Some("Ubuntu-22.04".into()),
            },
            program: "bash".into(),
            arguments: vec!["-l".into()],
            working_directory: Some(PathBuf::from("/home/user/dev/muxtrix")),
        };

        let plan = LaunchPlan::from_profile(&profile)?;
        assert_eq!(plan.executable, "wsl.exe");
        assert_eq!(
            plan.arguments,
            [
                "--distribution",
                "Ubuntu-22.04",
                "--cd",
                "/home/user/dev/muxtrix",
                "--exec",
                "bash",
                "-l",
            ]
        );
        assert_eq!(plan.working_directory, None);
        assert!(plan.environment.is_empty());
        Ok(())
    }

    #[test]
    fn wsl_profile_uses_the_distribution_default_shell_and_home() -> Result<(), PlatformError> {
        let profile = LaunchProfile {
            id: ProfileId::new(),
            name: "Default WSL shell".into(),
            backend: ProcessBackend::Wsl {
                distribution: Some("Ubuntu-24.04".into()),
            },
            program: String::new(),
            arguments: Vec::new(),
            working_directory: Some(PathBuf::from("~")),
        };

        let plan = LaunchPlan::from_profile(&profile)?;
        assert_eq!(plan.executable, "wsl.exe");
        assert_eq!(
            plan.arguments,
            [
                "--distribution",
                "Ubuntu-24.04",
                "--cd",
                "~",
                "--shell-type",
                "login"
            ]
        );
        assert_eq!(plan.working_directory, None);
        Ok(())
    }
}
