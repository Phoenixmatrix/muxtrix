#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use iced::Task;
use iced::widget::scrollable;

mod agent_screen;
mod agents_roster;
mod app;
mod box_drawing;
mod commands;
mod doctor;
mod effect;
mod ellipsized_text;
mod geom;
mod github;
mod input;
mod layout;
mod metrics;
mod popover;
mod process;
mod settings;
mod terminal;
mod terminal_image;
mod theme;
mod themes;
mod views;

#[cfg(feature = "e2e")]
mod e2e;

use app::{
    GITHUB_FILE_SCROLL_ID, GITHUB_KEYBOARD_SINK_ID, GITHUB_PULL_REQUEST_QUERY_ID,
    GITHUB_PULL_REQUEST_SCROLL_ID, Message, Muxtrix, NO_TERMINAL_STARTUP, PALETTE_INPUT_ID,
    PALETTE_SCROLL_ID, RENAME_INPUT_ID, SETTINGS_SCROLL_ID, WORKSPACE_CREATE_INPUT_ID,
    WORKTREE_INPUT_ID,
};
use effect::{Effect, FocusTarget, ScrollTarget};
use settings::AppSettings;
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

pub fn main() -> iced::Result {
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

    let (startup_settings, _) = AppSettings::load();
    iced::application(
        || {
            let (app, effects) = Muxtrix::boot();
            (app, run_effects(effects))
        },
        |app: &mut Muxtrix, message| run_effects(app.update(message)),
        Muxtrix::view,
    )
    .title(Muxtrix::title)
    .theme(Muxtrix::theme)
    .subscription(Muxtrix::subscription)
    .window(muxtrix_window_settings())
    .settings(iced::Settings {
        antialiasing: true,
        default_font: startup_settings.ui_font(),
        ..iced::Settings::default()
    })
    .run()
}

fn muxtrix_window_settings() -> iced::window::Settings {
    let mut settings = iced::window::Settings {
        size: iced::Size::new(1_280.0, 800.0),
        min_size: Some(iced::Size::new(720.0, 480.0)),
        icon: muxtrix_window_icon(),
        ..iced::window::Settings::default()
    };
    #[cfg(target_os = "linux")]
    {
        // Match muxtrix.desktop so Wayland compositors associate the window
        // with the installed launcher and its hicolor icon.
        settings.platform_specific.application_id = "muxtrix".into();
    }
    settings
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

fn muxtrix_window_icon() -> Option<iced::window::Icon> {
    iced::window::icon::from_rgba(
        include_bytes!("../assets/muxtrix-icon.rgba").to_vec(),
        256,
        256,
    )
    .ok()
}

/// The iced half of the effect protocol.
///
/// `update` decides *what* should happen and says so as data; this is the only
/// place that knows how to make it happen in iced's terms. Swapping the UI
/// framework means rewriting this function, not the eight thousand lines of
/// logic behind it.
fn run_effects(effects: Vec<Effect>) -> Task<Message> {
    Task::batch(effects.into_iter().map(run_effect))
}

fn focus_widget_id(target: FocusTarget) -> iced::widget::Id {
    iced::widget::Id::new(match target {
        FocusTarget::CommandPalette => PALETTE_INPUT_ID,
        FocusTarget::WorkspaceCreate => WORKSPACE_CREATE_INPUT_ID,
        FocusTarget::Rename => RENAME_INPUT_ID,
        FocusTarget::Worktree => WORKTREE_INPUT_ID,
        FocusTarget::GitHubPullRequestQuery => GITHUB_PULL_REQUEST_QUERY_ID,
        FocusTarget::GitHubKeyboardSink => GITHUB_KEYBOARD_SINK_ID,
    })
}

fn scroll_widget_id(target: ScrollTarget) -> iced::widget::Id {
    iced::widget::Id::new(match target {
        ScrollTarget::Settings => SETTINGS_SCROLL_ID,
        ScrollTarget::CommandPalette => PALETTE_SCROLL_ID,
        ScrollTarget::GitHubFiles => GITHUB_FILE_SCROLL_ID,
        ScrollTarget::GitHubPullRequests => GITHUB_PULL_REQUEST_SCROLL_ID,
    })
}

fn run_effect(effect: Effect) -> Task<Message> {
    match effect {
        Effect::Perform(work) => {
            // Kept off the executor: this work shells out to git and gh, and
            // blocking an async worker on a subprocess stalls every other
            // pending task. The closure is handed back if the thread cannot
            // start, so the caller still gets its answer rather than waiting
            // on a message that will never arrive.
            let (sender, receiver) = async_channel::bounded(1);
            let work = Arc::new(Mutex::new(Some(work)));
            let claimed = Arc::clone(&work);
            let spawned = std::thread::Builder::new()
                .name("muxtrix-effect".into())
                .spawn(move || {
                    let job = claimed.lock().ok().and_then(|mut slot| slot.take());
                    if let Some(job) = job {
                        let _ = sender.send_blocking(job());
                    }
                });
            if spawned.is_err() {
                let job = work.lock().ok().and_then(|mut slot| slot.take());
                return job.map_or_else(Task::none, |job| Task::done(job()));
            }
            // The channel carries exactly one message and then closes, so
            // running it as a stream also covers the thread dying early.
            Task::run(receiver, |message| message)
        }
        Effect::Focus(target) => iced::widget::operation::focus(focus_widget_id(target)),
        Effect::ScrollToRatio(target, ratio) => iced::widget::operation::snap_to(
            scroll_widget_id(target),
            iced::widget::operation::RelativeOffset { x: 0.0, y: ratio },
        ),
        Effect::ScrollToOffset(target, offset) => iced::widget::operation::scroll_to(
            scroll_widget_id(target),
            scrollable::AbsoluteOffset { x: 0.0, y: offset },
        ),
        #[cfg(feature = "e2e")]
        Effect::ScrollToEnd(target) => {
            iced::widget::operation::snap_to_end(scroll_widget_id(target))
        }
        Effect::ClipboardWrite(text) => iced::clipboard::write(text),
        Effect::ClipboardRead(map) => iced::clipboard::read().map(move |contents| map(contents)),
        Effect::SetResizeIncrements(increments) => iced::window::latest().then(move |window| {
            window.map_or_else(Task::none, |window| {
                iced::window::set_resize_increments(window, Some(increments.into()))
            })
        }),
        #[cfg(feature = "e2e")]
        Effect::Capture => iced::window::latest().then(|window| match window {
            Some(window) => iced::window::screenshot(window).map(Message::E2eScreenshot),
            None => Task::done(Message::E2eWindowMissing),
        }),
        #[cfg(feature = "e2e")]
        Effect::Exit => iced::exit(),
    }
}
