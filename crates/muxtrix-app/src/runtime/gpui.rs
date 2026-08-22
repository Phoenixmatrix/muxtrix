//! The GPUI runtime: window, event loop, timers, and the effect runner.
//!
//! Sibling of [`super::iced`]. The application core is identical under both —
//! `update` returns [`Effect`] values and this module decides what they mean
//! to GPUI — so the framework swap is contained in this file plus the elements
//! it draws with.

use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    ParentElement, Point, Render, Styled, Window, WindowBounds, WindowOptions, div, point, px,
    size,
};

use crate::app::{Message, Muxtrix};
use crate::effect::Effect;
use crate::input::KeyEvent;
use crate::theme::DesignTokens;

/// How often the cursor blink phase flips.
const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// The GitHub panel's loading animation step.
const GITHUB_LOADING_INTERVAL: Duration = Duration::from_millis(90);
/// The e2e harness's scenario tick.
#[cfg(feature = "e2e")]
const E2E_INTERVAL: Duration = Duration::from_millis(50);

/// Start the application and run it until the window closes.
pub(crate) fn run() -> iced::Result {
    gpui_platform::application()
        .with_assets(crate::assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);

            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(720.), px(480.))),
                    app_id: Some("muxtrix".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Root::new(window, cx)),
            );
            match opened {
                Ok(_) => cx.activate(true),
                Err(error) => {
                    eprintln!("muxtrix: could not open a window: {error}");
                    cx.quit();
                }
            }
        });
    Ok(())
}

/// The window's root view.
///
/// Owns the single [`Muxtrix`] value and turns messages into state changes and
/// effects — what `iced::application` does for the other runtime.
pub(crate) struct Root {
    app: Muxtrix,
    /// The window's keyboard focus. Held by the root rather than by any
    /// surface: Muxtrix routes every key through one handler and decides there
    /// whether it belongs to a shortcut, a dialog or the terminal, and that
    /// decision has to keep working before the inputs of Phase 4 exist.
    focus: FocusHandle,
}

impl Root {
    /// The application state the views read.
    pub(crate) fn app(&self) -> &Muxtrix {
        &self.app
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (app, effects) = Muxtrix::boot();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut root = Self { app, focus };
        root.spawn_timers(cx);
        root.spawn_terminal_wakeups(cx);
        root.observe_window(window, cx);
        root.run_effects(effects, window, cx);
        // The startup terminal is launched by `WindowOpened`, which iced
        // delivered as a window event. There is a window by the time this
        // runs, so say so directly.
        let bounds = window.bounds();
        let size = crate::geom::Size::new(bounds.size.width.into(), bounds.size.height.into());
        root.dispatch(Message::WindowOpened(size), window, cx);
        root
    }

    /// Apply one message and carry out whatever it asks for.
    pub(crate) fn dispatch(
        &mut self,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let effects = self.app.update(message);
        self.run_effects(effects, window, cx);
        window.set_window_title(&self.app.title());
        cx.notify();
    }

    /// Deliver a message from a context that has no `Window` — a timer or a
    /// channel. The window is reached through the entity's own handle.
    pub(crate) fn dispatch_detached(&mut self, message: Message, cx: &mut Context<Self>) {
        let effects = self.app.update(message);
        self.run_detached_effects(effects, cx);
        cx.notify();
    }

    fn run_effects(&mut self, effects: Vec<Effect>, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_detached_effects(effects, cx);
    }

    fn run_detached_effects(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            self.run_effect(effect, cx);
        }
    }

    /// Carry out one effect.
    ///
    /// Focus and scroll are deliberately inert until Phase 4 brings the inputs
    /// and lists they name; there is nothing to focus or scroll yet, and a
    /// placeholder handle would be a lie rather than a step.
    fn run_effect(&mut self, effect: Effect, cx: &mut Context<Self>) {
        match effect {
            Effect::Perform(work) => {
                cx.spawn(async move |this, cx| {
                    // Off the UI thread: this work shells out to git and gh.
                    let message = cx.background_executor().spawn(async move { work() }).await;
                    let _ = this.update(cx, |root, cx| root.dispatch_detached(message, cx));
                })
                .detach();
            }
            Effect::ClipboardWrite(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            Effect::ClipboardRead(map) => {
                let contents = cx.read_from_clipboard().and_then(|item| item.text());
                self.dispatch_detached(map(contents), cx);
            }
            // GPUI has no resize-increment API. Documented as an accepted loss
            // in docs/GPU.md; WSLg and Wayland lose whole-cell resizing.
            Effect::SetResizeIncrements(_) => {}
            Effect::Focus(_) | Effect::ScrollToRatio(..) | Effect::ScrollToOffset(..) => {}
            #[cfg(feature = "e2e")]
            Effect::ScrollToEnd(_) => {}
            #[cfg(feature = "e2e")]
            Effect::Capture => {}
            #[cfg(feature = "e2e")]
            Effect::Exit => cx.quit(),
        }
    }

    /// The periodic messages the iced runtime got from `subscription`.
    ///
    /// Each is its own loop rather than one tick with counters, so the gating
    /// conditions stay where they are readable and a paused animation costs
    /// nothing but a sleeping task.
    fn spawn_timers(&mut self, cx: &mut Context<Self>) {
        repeat(cx, BLINK_INTERVAL, |_| Some(Message::BlinkCursor));
        repeat(cx, GITHUB_LOADING_INTERVAL, |root| {
            root.app
                .github_loading_animating()
                .then_some(Message::AnimateGitHubLoading)
        });
        repeat(cx, Duration::from_millis(1), |root| {
            root.app
                .github_pull_requests_refresh_pending
                .then_some(Message::RefreshGitHubPullRequestsAfterAgentTurn)
        });
        #[cfg(feature = "e2e")]
        repeat(cx, E2E_INTERVAL, |root| {
            root.app.has_e2e_scenario().then_some(Message::E2eTick)
        });
    }

    /// The terminal actors signal readable output on a channel; each signal is
    /// one poll. This replaces the iced `Subscription::run_with` stream.
    fn spawn_terminal_wakeups(&mut self, cx: &mut Context<Self>) {
        let receiver = self.app.event_receiver.clone();
        cx.spawn(async move |this, cx| {
            while receiver.recv().await.is_ok() {
                if this
                    .update(cx, |root, cx| {
                        root.dispatch_detached(Message::PollTerminal, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Every key the window receives, in the app's own vocabulary.
    ///
    /// The rule the iced runtime enforced through `app_event` carries over:
    /// the terminal receives everything the app does not explicitly claim, so
    /// there is no keymap here to consult first.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let input = crate::input::from_keystroke(&event.keystroke);
        self.dispatch(Message::Keyboard(KeyEvent::Pressed(input)), window, cx);
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = crate::input::modifiers_from_gpui(event.keystroke.modifiers);
        self.dispatch(
            Message::Keyboard(KeyEvent::Released { modifiers }),
            window,
            cx,
        );
    }

    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = crate::input::modifiers_from_gpui(event.modifiers);
        self.dispatch(
            Message::Keyboard(KeyEvent::ModifiersChanged(modifiers)),
            window,
            cx,
        );
    }

    fn observe_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.observe_window_activation(window, |root, window, cx| {
            let focused = window.is_window_active();
            root.dispatch(Message::WindowFocusChanged(focused), window, cx);
        })
        .detach();
        cx.observe_window_bounds(window, |root, window, cx| {
            let bounds = window.bounds();
            let size = crate::geom::Size::new(bounds.size.width.into(), bounds.size.height.into());
            root.dispatch(Message::WindowResized(size), window, cx);
        })
        .detach();
    }
}

/// Run `message` on an interval for as long as the view lives.
///
/// The closure is asked each tick rather than once, so a message that should
/// only fire under some condition (a loading animation, an e2e run) can stop
/// producing without tearing the timer down.
fn repeat<F>(cx: &mut Context<Root>, every: Duration, message: F)
where
    F: Fn(&Root) -> Option<Message> + 'static,
{
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(every).await;
            let stopped = this
                .update(cx, |root, cx| {
                    if let Some(message) = message(root) {
                        root.dispatch_detached(message, cx);
                    }
                })
                .is_err();
            if stopped {
                break;
            }
        }
    })
    .detach();
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = DesignTokens::for_appearance(self.app.settings.appearance);
        let workspace = self.view_workspace(window, cx);
        // The menu floats above the shell and any press outside it dismisses
        // and is consumed — the behaviour the iced `Popover` had, and what
        // `pane_menu_click_away_observed` asserts.
        let menu = self.pane_menu(cx).map(|menu| {
            div()
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::DismissPaneMenu, window, cx);
                    }),
                )
                .child(
                    gpui::deferred(
                        gpui::anchored()
                            .position(root_menu_anchor(window))
                            .anchor(gpui::Anchor::TopRight)
                            .snap_to_window_with_margin(px(6.))
                            .child(menu),
                    )
                    .with_priority(1),
                )
        });
        div()
            .track_focus(&self.focus)
            .key_context("Muxtrix")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .size_full()
            .flex()
            .flex_col()
            .bg(color(tokens.app))
            .text_color(color(tokens.text))
            .child(workspace)
            .children(menu)
    }
}

impl Focusable for Root {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Bridge a design token to GPUI's colour type.
///
/// [`DesignTokens`] stays the single source of truth for chrome colour under
/// both runtimes; only the conversion differs.
pub(crate) fn color(value: iced::Color) -> gpui::Rgba {
    gpui::Rgba {
        r: value.r,
        g: value.g,
        b: value.b,
        a: value.a,
    }
}

/// Where the pane menu hangs from.
///
/// The iced popover anchored to the overflow button, 6 px in and 38 px below
/// its top. GPUI elements do not report their painted bounds back to the view
/// that built them, so this places the menu against the window's top-right
/// instead, which is where that button sits in every layout the header has.
fn root_menu_anchor(window: &Window) -> Point<gpui::Pixels> {
    let bounds = window.bounds();
    point(bounds.size.width - px(6.), px(38.))
}
