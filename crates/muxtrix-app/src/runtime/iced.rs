//! The iced runtime: window setup and the effect runner.
//!
//! Everything iced-specific that is not a view lives here. `update` decides
//! what should happen and says so as [`Effect`] values; this module is the only
//! code that knows how to carry them out in iced's terms, which is what makes
//! swapping the framework a matter of writing a sibling of this file rather
//! than touching the application.

use std::sync::{Arc, Mutex};

use iced::Task;
use iced::widget::scrollable;

use crate::app::{
    GITHUB_FILE_SCROLL_ID, GITHUB_KEYBOARD_SINK_ID, GITHUB_PULL_REQUEST_QUERY_ID,
    GITHUB_PULL_REQUEST_SCROLL_ID, Message, Muxtrix, PALETTE_INPUT_ID, PALETTE_SCROLL_ID,
    RENAME_INPUT_ID, SETTINGS_SCROLL_ID, WORKSPACE_CREATE_INPUT_ID, WORKTREE_INPUT_ID,
};
use crate::effect::{Effect, FocusTarget, ScrollTarget};
use crate::settings::AppSettings;

/// Start the application and run it until the window closes.
pub(crate) fn run() -> iced::Result {
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

pub(crate) fn muxtrix_window_settings() -> iced::window::Settings {
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

pub(crate) fn muxtrix_window_icon() -> Option<iced::window::Icon> {
    iced::window::icon::from_rgba(
        include_bytes!("../../assets/muxtrix-icon.rgba").to_vec(),
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
        Effect::Exit => iced::exit(),
    }
}
