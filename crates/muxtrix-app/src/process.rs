//! Console subprocesses that stay out of sight on Windows.

#[cfg(not(target_os = "windows"))]
use std::process::Child;
use std::{
    ffi::OsStr,
    io::{self, Read},
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// A command for a console program, windowless on Windows.
///
/// The GUI binary carries no console of its own, so Windows hands every
/// console child a brand new console window unless `CREATE_NO_WINDOW` says
/// otherwise — one visible flash per `gh`, `git`, `wsl.exe`, or agent poll.
/// Every short-lived helper the app shells out to goes through here.
#[cfg(target_os = "windows")]
pub(crate) fn console_command(program: impl AsRef<OsStr>) -> Command {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// A command for a console program; only Windows needs the window suppressed.
#[cfg(not(target_os = "windows"))]
pub(crate) fn console_command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

/// Cooperative cancellation for a short-lived console command.
///
/// The command runner checks this flag while waiting and terminates the child
/// process before returning. Clones refer to the same request.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessCancellation {
    cancelled: Arc<AtomicBool>,
}

impl ProcessCancellation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

const COMMAND_STDOUT_MAX_BYTES: usize = 64 * 1024 * 1024;
const COMMAND_STDERR_MAX_BYTES: usize = 1024 * 1024;

/// Captures a command's output without allowing it to outlive its request.
///
/// Stdout and stderr are drained concurrently so a verbose child cannot fill a
/// pipe and deadlock before the timeout or cancellation check runs.
pub(crate) fn command_output(
    command: &mut Command,
    timeout: Duration,
    cancellation: &ProcessCancellation,
) -> io::Result<Output> {
    let captured = command_output_limited(
        command,
        timeout,
        cancellation,
        Some(COMMAND_STDOUT_MAX_BYTES),
        Some(COMMAND_STDERR_MAX_BYTES),
    )?;
    if captured.stdout_truncated {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "command stdout exceeded {} MiB",
                COMMAND_STDOUT_MAX_BYTES / (1024 * 1024)
            ),
        ));
    }
    Ok(captured.output)
}

pub(crate) struct LimitedOutput {
    pub(crate) output: Output,
    pub(crate) stdout_truncated: bool,
}

/// Captures output while retaining at most the requested bytes from each pipe.
///
/// Readers continue draining after their storage limit so the child can always
/// exit cleanly; `stdout_truncated` tells callers not to parse partial content.
pub(crate) fn command_output_limited(
    command: &mut Command,
    timeout: Duration,
    cancellation: &ProcessCancellation,
    stdout_limit: Option<usize>,
    stderr_limit: Option<usize>,
) -> io::Result<LimitedOutput> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn_managed(command)?;
    #[cfg(not(target_os = "windows"))]
    let stdout = child.stdout.take();
    #[cfg(target_os = "windows")]
    let stdout = child.stdout().take();
    let Some(stdout) = stdout else {
        terminate_process_tree(&mut child);
        return Err(io::Error::other("child stdout was not captured"));
    };
    #[cfg(not(target_os = "windows"))]
    let stderr = child.stderr.take();
    #[cfg(target_os = "windows")]
    let stderr = child.stderr().take();
    let Some(stderr) = stderr else {
        drop(stdout);
        terminate_process_tree(&mut child);
        return Err(io::Error::other("child stderr was not captured"));
    };
    let stdout_reader = match thread::Builder::new()
        .name("muxtrix-command-stdout".into())
        .spawn(move || drain_output(stdout, stdout_limit))
    {
        Ok(reader) => reader,
        Err(error) => {
            drop(stderr);
            terminate_process_tree(&mut child);
            return Err(error);
        }
    };
    let stderr_reader = match thread::Builder::new()
        .name("muxtrix-command-stderr".into())
        .spawn(move || drain_output(stderr, stderr_limit))
    {
        Ok(reader) => reader,
        Err(error) => {
            terminate_process_tree(&mut child);
            let _ = stdout_reader.join();
            return Err(error);
        }
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        if cancellation.is_cancelled() {
            break Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "command was cancelled",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("command timed out after {} seconds", timeout.as_secs()),
                ));
            }
            Err(error) => break Err(error),
        }
    };
    terminate_process_tree(&mut child);
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| io::Error::other("stdout reader panicked"))??;
    let (stderr, _) = stderr_reader
        .join()
        .map_err(|_| io::Error::other("stderr reader panicked"))??;
    Ok(LimitedOutput {
        output: Output {
            status: status?,
            stdout,
            stderr,
        },
        stdout_truncated,
    })
}

#[cfg(not(target_os = "windows"))]
type ManagedChild = Child;

#[cfg(target_os = "windows")]
type ManagedChild = Box<dyn process_wrap::std::ChildWrapper>;

#[cfg(not(target_os = "windows"))]
fn spawn_managed(command: &mut Command) -> io::Result<ManagedChild> {
    command.spawn()
}

#[cfg(target_os = "windows")]
fn spawn_managed(command: &mut Command) -> io::Result<ManagedChild> {
    use process_wrap::std::{CommandWrap, CreationFlags, JobObject};
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;

    let owned = std::mem::replace(command, Command::new(""));
    let mut command = CommandWrap::from(owned);
    command.wrap(CreationFlags(CREATE_NO_WINDOW));
    command.wrap(JobObject);
    command.spawn()
}

fn terminate_process_tree(child: &mut ManagedChild) {
    #[cfg(unix)]
    {
        let process_id = child.id().to_string();
        let _ = console_command("/bin/kill")
            .args(["-KILL", "--", &format!("-{process_id}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn drain_output(
    mut reader: impl Read,
    storage_limit: Option<usize>,
) -> io::Result<(Vec<u8>, bool)> {
    let mut stored = Vec::with_capacity(storage_limit.unwrap_or(8 * 1024).min(256 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let keep = storage_limit
            .map(|limit| read.min(limit.saturating_sub(stored.len())))
            .unwrap_or(read);
        stored.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    Ok((stored, truncated))
}

#[cfg(test)]
mod tests {
    use super::{ProcessCancellation, command_output, console_command, drain_output};
    use std::time::{Duration, Instant};

    #[test]
    fn diagnostic_drain_stores_only_its_declared_limit() {
        let bytes = vec![b'x'; 128 * 1024];
        let (stored, truncated) =
            drain_output(bytes.as_slice(), Some(64 * 1024)).expect("drain should succeed");
        assert_eq!(stored.len(), 64 * 1024);
        assert!(truncated);
    }
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn command_output_stops_at_its_deadline() {
        let mut command = console_command("sleep");
        command.arg("5");
        let started = Instant::now();
        let error = command_output(
            &mut command,
            Duration::from_millis(30),
            &ProcessCancellation::default(),
        )
        .expect_err("sleep should time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn command_output_reaps_successful_commands_background_descendants() {
        let mut command = console_command("sh");
        command.args(["-c", "sleep 5 &"]);
        let started = Instant::now();
        let output = command_output(
            &mut command,
            Duration::from_secs(5),
            &ProcessCancellation::default(),
        )
        .expect("shell should exit successfully");
        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn command_output_honors_cooperative_cancellation() {
        let cancellation = ProcessCancellation::default();
        let cancel_from_thread = cancellation.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            cancel_from_thread.cancel();
        });
        let mut command = console_command("sh");
        command.args(["-c", "sleep 5 & wait"]);
        let started = Instant::now();
        let error = command_output(&mut command, Duration::from_secs(5), &cancellation)
            .expect_err("sleep should be cancelled");
        canceller.join().expect("canceller should finish");
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
