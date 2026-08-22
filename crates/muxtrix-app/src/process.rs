//! Console subprocesses that stay out of sight on Windows.

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

/// Ceiling for the short-lived helpers the app shells out to outside the
/// GitHub paths, which carry timeouts of their own.
///
/// Generous enough that a cold WSL distribution still answers, and short
/// enough that a wedged one cannot hold a thread — and the console Windows
/// gave the command — for the lifetime of the process.
pub(crate) const HELPER_COMMAND_TIMEOUT: Duration = Duration::from_secs(2 * 60);

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
    // `console_command` owns the Windows creation flags. Spawn that same
    // `std::process::Command` directly: wrapping or rebuilding it here is where
    // the v0.1.56 path diverged from the known-windowless runner.
    let mut child = command.spawn()?;
    // Adopted before anything else touches the child: from here on every exit
    // from this function ends the whole tree, including the `?` paths.
    let mut tree = ProcessTree::adopt(&child);
    let stdout = child.stdout.take();
    let Some(stdout) = stdout else {
        terminate_process_tree(&mut child, &mut tree);
        return Err(io::Error::other("child stdout was not captured"));
    };
    let stderr = child.stderr.take();
    let Some(stderr) = stderr else {
        drop(stdout);
        terminate_process_tree(&mut child, &mut tree);
        return Err(io::Error::other("child stderr was not captured"));
    };
    let stdout_reader = match thread::Builder::new()
        .name("muxtrix-command-stdout".into())
        .spawn(move || drain_output(stdout, stdout_limit))
    {
        Ok(reader) => reader,
        Err(error) => {
            drop(stderr);
            terminate_process_tree(&mut child, &mut tree);
            return Err(error);
        }
    };
    let stderr_reader = match thread::Builder::new()
        .name("muxtrix-command-stderr".into())
        .spawn(move || drain_output(stderr, stderr_limit))
    {
        Ok(reader) => reader,
        Err(error) => {
            // Terminating first is deliberate: this join waits on the very pipe
            // a surviving descendant would be holding.
            terminate_process_tree(&mut child, &mut tree);
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
    terminate_process_tree(&mut child, &mut tree);
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

/// Ownership of a child process *and everything it goes on to spawn*.
///
/// Unix has the process group, which `terminate_process_tree` signals directly.
/// Windows has no equivalent, so `Child::kill` there reaches the program that
/// was launched and none of its own children — a credential helper, `ssh.exe`,
/// a remote helper. Survivors cost twice over: each holds open the console the
/// command was given, which on Windows is a `conhost.exe` that then outlives
/// everything, and each holds the pipes this runner is draining, so the reader
/// joins below would never return.
///
/// A job object is the tree Windows does understand.
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` ties every descendant's lifetime to the
/// handle, so closing it ends the whole tree — after a clean exit, a timeout, a
/// cancellation, or an unwind straight past this value.
#[cfg(target_os = "windows")]
struct ProcessTree(Option<win32job::Job>);

#[cfg(target_os = "windows")]
impl ProcessTree {
    /// Adopts an already-spawned child.
    ///
    /// A job the platform declines to create or assign leaves the runner
    /// exactly where it stood before — direct child only — rather than failing
    /// a command over how its cleanup would have worked. The gap between
    /// `spawn` and the assignment is the one thing this cannot cover: a
    /// grandchild started inside that window is born outside the job. It is
    /// microseconds wide, and `git`, `gh`, and `wsl.exe` all load and parse
    /// before they launch anything.
    fn adopt(child: &Child) -> Self {
        use std::os::windows::io::AsRawHandle as _;

        let mut limits = win32job::ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close();
        let Ok(job) = win32job::Job::create_with_limit_info(&limits) else {
            return Self(None);
        };
        if job.assign_process(child.as_raw_handle() as isize).is_err() {
            return Self(None);
        }
        Self(Some(job))
    }

    /// Ends every process still in the tree. Idempotent.
    ///
    /// Dropping the job is the kill: `limit_kill_on_job_close` above is what
    /// makes closing the last handle terminate everything still inside.
    fn close(&mut self) {
        drop(self.0.take());
    }
}

/// The tree is the process group here, and `terminate_process_tree` signals it
/// by id, so there is nothing for this to own.
#[cfg(not(target_os = "windows"))]
struct ProcessTree;

#[cfg(not(target_os = "windows"))]
impl ProcessTree {
    fn adopt(_child: &Child) -> Self {
        Self
    }

    fn close(&mut self) {}
}

fn terminate_process_tree(child: &mut Child, tree: &mut ProcessTree) {
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
    // Windows only: closing the job is what reaches the descendants `kill`
    // cannot, and every caller below goes on to join the pipe readers those
    // descendants would otherwise hold open forever.
    tree.close();
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

    /// The windowless guarantee lives in *where* commands are built, and until
    /// now nothing enforced it: `console_command_detaches_console_children`
    /// below is the behavioural proof, and it is `cfg(windows)` in a project
    /// whose CI builds Windows but only ever runs tests on Linux. This one runs
    /// everywhere, and it is what a raw `Command::new` trips over.
    #[test]
    fn console_programs_are_built_through_console_command() {
        // `process.rs` is where the flags are applied, and `gpu.rs` re-execs
        // this same binary through `exec` on Linux — it never spawns a child,
        // and never runs on Windows at all. A whole file named `tests.rs` is
        // fixtures end to end, the same exemption the inline `mod tests` below
        // gets.
        const EXEMPT: [&str; 3] = ["process.rs", "gpu.rs", "tests.rs"];

        let mut pending = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        let mut offenders = Vec::new();
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for path in entries.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs")
                    || path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| EXEMPT.contains(&name))
                {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };
                // Fixtures build their own commands; only shipped code is bound
                // by the rule.
                let shipped = source
                    .split("#[cfg(test)]\nmod tests {")
                    .next()
                    .unwrap_or_default();
                offenders.extend(
                    shipped
                        .lines()
                        .enumerate()
                        .filter(|(_, line)| line.contains("Command::new"))
                        .map(|(index, _)| format!("{}:{}", path.display(), index + 1)),
                );
            }
        }
        assert!(
            offenders.is_empty(),
            "build these with `console_command`, or Windows opens a console window for each: {offenders:?}"
        );
    }

    #[test]
    fn diagnostic_drain_stores_only_its_declared_limit() {
        let bytes = vec![b'x'; 128 * 1024];
        let (stored, truncated) =
            drain_output(bytes.as_slice(), Some(64 * 1024)).expect("drain should succeed");
        assert_eq!(stored.len(), 64 * 1024);
        assert!(truncated);
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn console_command_detaches_console_children() {
        let script = concat!(
            "$signature = '[DllImport(\"kernel32.dll\")] ",
            "public static extern IntPtr GetConsoleWindow();'; ",
            "Add-Type -MemberDefinition $signature -Name NativeMethods -Namespace Win32; ",
            "if ([Win32.NativeMethods]::GetConsoleWindow() -ne [IntPtr]::Zero) { exit 1 }"
        );
        let output = console_command("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .output()
            .expect("console probe should run");
        assert!(
            output.status.success(),
            "console child inherited or created a console: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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

    /// The Windows half of
    /// `command_output_reaps_successful_commands_background_descendants`.
    ///
    /// `cmd` exits at once and leaves `ping` running with the inherited stdout
    /// pipe. Without a job object the drain below never reaches EOF and this
    /// call never returns — and the console Windows created for the command
    /// stays alive for as long as the survivor does.
    #[cfg(target_os = "windows")]
    #[test]
    fn command_output_reaps_successful_commands_background_descendants() {
        let mut command = console_command("cmd.exe");
        command.args(["/c", "start /b ping -n 30 127.0.0.1"]);
        let started = Instant::now();
        let output = command_output(
            &mut command,
            Duration::from_secs(20),
            &ProcessCancellation::default(),
        )
        .expect("cmd should exit successfully");
        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(10));
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
