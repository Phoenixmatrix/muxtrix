#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

mod agent_screen;
mod agents_roster;
mod app;
mod assets;
mod box_drawing;
mod commands;
mod doctor;
mod effect;
mod geom;
mod github;
mod input;
mod layout;
mod metrics;
mod process;
mod runtime;
mod settings;
mod terminal;
mod terminal_image;
mod theme;
mod themes;
mod views;

#[cfg(feature = "e2e")]
mod e2e;

use app::NO_TERMINAL_STARTUP;
/*
THESIS
Muxtrix is a calm native control room for many concurrent terminals and coding agents.

OWN-WORLD
Its visual world is a dark live gate board: ruled fleet entries, precise signals, quiet chrome,
and terminal content as the dominant surface.

STORY
See global exceptions only when present; scan every task and agent state; jump directly to work;
operate each pane from its own compact tab; tune the environment without leaving the app.

FIRST VIEWPORT
At launch, the fleet rail and active terminal fill the window. No dashboard, inbox, or decorative
status chrome competes with live work.

FORM
Category standard, chosen via canon. Direction seed: eb790489.

FINISH
unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, and DESIGN.md
*/

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Session daemon mode: no window, no GPU — just PTYs on a socket. It
    // lives inside this binary so packages ship no extra executable.
    let arguments: Vec<String> = std::env::args().collect();
    if matches!(arguments.as_slice(), [_, flag] if flag == "--version" || flag == "-V") {
        println!("muxtrix {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if arguments.iter().any(|argument| argument == "--sessiond") {
        run_session_daemon(&arguments);
        return Ok(());
    }
    if let Err(error) = muxtrix::gpu::ensure_wsl_gpu_defaults() {
        eprintln!("failed to apply process-local WSL GPU defaults: {error}");
    }
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "doctor")
    {
        if let Err(code) = doctor::run(&arguments[2..]) {
            std::process::exit(code);
        }
        return Ok(());
    }
    NO_TERMINAL_STARTUP.store(no_terminal_requested(&arguments), Ordering::Relaxed);

    runtime::run()
}

fn no_terminal_requested(arguments: &[String]) -> bool {
    arguments.iter().any(|argument| argument == "--no-terminal")
}

fn run_session_daemon(arguments: &[String]) {
    let value_of = |flag: &str| {
        arguments
            .iter()
            .position(|argument| argument == flag)
            .and_then(|index| arguments.get(index + 1))
            .cloned()
    };
    let Some(id) = value_of("--session-id").and_then(|raw| raw.parse().ok()) else {
        eprintln!("--sessiond requires --session-id <uuid>");
        std::process::exit(2);
    };
    let name = value_of("--session-name").unwrap_or_else(|| "session".into());
    let endpoint =
        value_of("--session-endpoint").unwrap_or_else(|| muxtrix_sessions::session_endpoint(id));
    if let Err(error) = muxtrix_sessions::daemon::run(id, name, endpoint) {
        eprintln!("session daemon failed: {error}");
        std::process::exit(1);
    }
}

