//! Console subprocesses that stay out of sight on Windows.

use std::ffi::OsStr;
use std::process::Command;

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
