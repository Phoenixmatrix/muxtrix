//! What `update` asks the runtime to do, as data.
//!
//! `update` is otherwise a pure function of state and message, and keeping it
//! that way is what makes it testable without standing up a window: a test
//! asserts on the [`Effect`]s a message produced rather than on something that
//! only a running UI framework can inspect. The runtime is the only place that
//! knows how to carry them out.

use std::sync::Arc;

use crate::app::Message;

/// A side effect `update` wants performed.
pub(crate) enum Effect {
    /// Run blocking work off the UI thread; its return value becomes a message.
    Perform(Box<dyn FnOnce() -> Message + Send + 'static>),
    Focus(FocusTarget),
    /// Scroll to a fraction of the scrollable's length, 0.0 top to 1.0 end.
    ScrollToRatio(ScrollTarget, f32),
    /// Scroll to an absolute offset in pixels from the top.
    ScrollToOffset(ScrollTarget, f32),
    ClipboardWrite(String),
    /// Read the clipboard and turn its contents into a message.
    ///
    /// `Arc<dyn Fn>` rather than a boxed `FnOnce` because the runtime may hand
    /// the closure to a callback it can only hold by shared reference.
    ClipboardRead(Arc<dyn Fn(Option<String>) -> Message + Send + Sync + 'static>),
    /// Scroll all the way to the end. Distinct from a 1.0 ratio: the end of a
    /// list whose length is still settling is not the same position.
    #[cfg(feature = "e2e")]
    ScrollToEnd(ScrollTarget),
    /// Grab the window's pixels. Only the e2e harness asks for this.
    #[cfg(feature = "e2e")]
    Capture,
    /// Quit the application. Reached when the control socket asks for it; a
    /// user closing the window goes through the platform instead.
    Exit,
}

/// A focusable surface, named by role rather than by widget id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusTarget {
    CommandPalette,
    WorkspaceCreate,
    Rename,
    Worktree,
    GitHubPullRequestQuery,
    /// Not a real widget. Focusing it moves focus off whatever held it, so
    /// subsequent list keys reach the GitHub panel handler instead of staying
    /// captured by a text editor.
    GitHubKeyboardSink,
}

/// A scrollable surface, named by role rather than by widget id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollTarget {
    Settings,
    CommandPalette,
    GitHubFiles,
    GitHubPullRequests,
}

/// Flatten several effect lists into one.
///
/// The shape `update` arms reach for when a message has more than one
/// consequence.
pub(crate) fn batch<I>(effects: I) -> Vec<Effect>
where
    I: IntoIterator<Item = Vec<Effect>>,
{
    effects.into_iter().flatten().collect()
}
