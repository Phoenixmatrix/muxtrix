//! Cross-platform process launch planning and PTY ownership.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use muxtrix_domain::{LaunchProfile, ProcessBackend};
use portable_pty::{Child, CommandBuilder, MasterPty, native_pty_system};
use thiserror::Error;

pub use portable_pty::PtySize;

/// PTY bytes with immutable provenance, preserved until the terminal consumes
/// them. Replaying history must rebuild the screen without answering old queries.
#[derive(Debug)]
pub enum PtyOutput {
    Live(Vec<u8>),
    Backlog(Vec<u8>),
}

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
    pub const BASH_PROMPT_COMMAND: &str = r#"printf '\033]7;file://%s%s\033\\' "$HOSTNAME" "$PWD""#;

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
    master: Option<Box<dyn MasterPty + Send>>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
    output_ended: Arc<AtomicBool>,
    #[cfg(target_os = "linux")]
    linux_session: Option<(u32, u64)>,
    termination_confirmed: bool,
}

struct ExitTrackingReader {
    reader: Box<dyn Read + Send>,
    ended: Arc<AtomicBool>,
}

impl Read for ExitTrackingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let result = self.reader.read(buffer);
        let eof = matches!(result, Ok(0)) && !buffer.is_empty();
        // Linux PTY masters report EIO, rather than a zero read, when the
        // last slave closes. Other read errors are not exit evidence.
        #[cfg(unix)]
        let eof = eof
            || result.as_ref().is_err_and(|error| {
                error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error())
            });
        if eof {
            self.ended.store(true, Ordering::Release);
        }
        result
    }
}

#[cfg(target_os = "linux")]
struct LinuxProcess {
    pid: u32,
    session: u32,
    started: u64,
    state: char,
}

#[cfg(target_os = "linux")]
fn linux_process(pid: u32) -> Result<Option<LinuxProcess>, PlatformError> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(PlatformError::Io(error.to_string())),
    };
    let invalid = || PlatformError::Pty(format!("cannot verify process {pid} ownership"));
    let (_, fields) = stat.rsplit_once(')').ok_or_else(invalid)?;
    let mut fields = fields.split_whitespace();
    let state = fields
        .next()
        .and_then(|state| state.chars().next())
        .ok_or_else(invalid)?;
    let session = fields
        .nth(2)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let started = fields
        .nth(15)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    Ok(Some(LinuxProcess {
        pid,
        session,
        started,
        state,
    }))
}

#[cfg(target_os = "linux")]
fn linux_session_members(session: u32, started: u64) -> Result<Vec<LinuxProcess>, PlatformError> {
    // The retained leader's birth time prevents a recycled PID from making
    // an unrelated, newer session look like this PTY's original owner.
    if linux_process(session)?.is_some_and(|leader| leader.started != started) {
        return Err(PlatformError::Pty(
            "PTY session leader identity has been reused".into(),
        ));
    }
    let mut members = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(|error| PlatformError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| PlatformError::Io(error.to_string()))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(process) = linux_process(pid)?
            && process.session == session
            && !matches!(process.state, 'Z' | 'X')
        {
            members.push(process);
        }
    }
    Ok(members)
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
        let output_ended = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "linux")]
        let linux_session = child.process_id().and_then(|pid| {
            linux_process(pid)
                .ok()
                .flatten()
                .filter(|process| process.session == pid)
                .map(|process| (pid, process.started))
        });

        Ok(Self {
            master: Some(pair.master),
            child,
            reader: Some(Box::new(ExitTrackingReader {
                reader,
                ended: Arc::clone(&output_ended),
            })),
            writer,
            output_ended,
            #[cfg(target_os = "linux")]
            linux_session,
            termination_confirmed: false,
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
            .as_ref()
            .ok_or_else(|| PlatformError::Pty("PTY has been closed".into()))?
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

    /// Task cleanup barrier: signaling alone is not evidence that the child
    /// has stopped using its working directory.
    pub fn kill_and_wait(&mut self) -> Result<(), PlatformError> {
        if self.termination_confirmed {
            return Ok(());
        }
        if cfg!(all(unix, not(target_os = "linux"))) {
            return Err(PlatformError::Pty(
                "cannot verify all PTY-owned process groups on this platform; keeping the task worktree (foreground-group exit alone is insufficient)".into(),
            ));
        }
        #[cfg(target_os = "linux")]
        let (session_id, session_started) = self.linux_session.ok_or_else(|| {
            PlatformError::Pty(
                "cannot establish the PTY's original process-session identity".into(),
            )
        })?;
        #[cfg(windows)]
        {
            // Closing ConPTY shuts down attached console clients, not just
            // its initial shell. Modern Windows returns before they exit;
            // the output EOF below is the documented completion barrier.
            if let Some(master) = self.master.take() {
                // Older Windows may block inside ClosePseudoConsole. Keep
                // that destructor off the bounded session-control path.
                std::thread::Builder::new()
                    .name("muxtrix-close-conpty".into())
                    .spawn(move || drop(master))
                    .map_err(|error| PlatformError::Pty(error.to_string()))?;
            }
        }
        #[cfg(not(any(unix, windows)))]
        if self.try_wait()?.is_none() {
            self.kill()?;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            #[cfg(target_os = "linux")]
            let members_gone = {
                use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
                let members = linux_session_members(session_id, session_started)?;
                let gone = members.is_empty();
                for member in members {
                    let pid = Pid::from_raw(member.pid as i32).ok_or_else(|| {
                        PlatformError::Pty("invalid PTY session member PID".into())
                    })?;
                    // Pin the kernel process before revalidating ownership.
                    // A later exit/PID reuse cannot redirect this signal.
                    let handle = match pidfd_open(pid, PidfdFlags::empty()) {
                        Ok(handle) => handle,
                        Err(rustix::io::Errno::SRCH) => continue,
                        Err(error) => {
                            return Err(PlatformError::Pty(format!(
                                "cannot safely identify PTY session member (pidfd required): {error}"
                            )));
                        }
                    };
                    if linux_process(member.pid)?.is_some_and(|current| {
                        current.session == session_id && current.started == member.started
                    }) {
                        match pidfd_send_signal(&handle, Signal::KILL) {
                            Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                            Err(error) => return Err(PlatformError::Pty(error.to_string())),
                        }
                    }
                }
                gone
            };
            #[cfg(not(target_os = "linux"))]
            let members_gone = true;
            if self.try_wait()?.is_some()
                && self.output_ended.load(Ordering::Acquire)
                && members_gone
            {
                self.termination_confirmed = true;
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(PlatformError::Pty(
                    "timed out waiting for child exit and PTY closure".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
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
