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
    pub(crate) app: Muxtrix,
    pub(crate) inputs: crate::views::gpui::inputs::Inputs,
    /// A focus request waiting for a frame to apply it against.
    pending_focus: Option<crate::effect::FocusTarget>,
    /// Inline terminal images, decoded once and kept while the emulator still
    /// references them. Keyed by the emulator's own generation, so an image
    /// that is redrawn unchanged is not decoded again.
    images: std::collections::BTreeMap<u64, std::sync::Arc<gpui::RenderImage>>,
    /// The window's keyboard focus. Held by the root rather than by any
    /// surface: Muxtrix routes every key through one handler and decides there
    /// whether it belongs to a shortcut, a dialog or the terminal, and that
    /// decision has to keep working before the inputs of Phase 4 exist.
    focus: FocusHandle,
}

impl Root {
    /// The images the terminals currently reference, decoded for GPUI.
    pub(crate) fn images(
        &self,
    ) -> &std::collections::BTreeMap<u64, std::sync::Arc<gpui::RenderImage>> {
        &self.images
    }

    /// Decode any new inline image and forget the ones no terminal shows.
    ///
    /// Keyed by the emulator's generation, which changes only when the image
    /// itself does, so a placement that merely moved costs nothing.
    fn sync_images(&mut self) {
        let mut live = std::collections::BTreeSet::new();
        for runtime in self.app.terminals.values() {
            let Some(snapshot) = runtime.snapshot.as_ref() else {
                continue;
            };
            for placement in &snapshot.images {
                let generation = placement.image.generation;
                live.insert(generation);
                self.images.entry(generation).or_insert_with(|| {
                    // GPUI wants BGRA; the emulator hands out RGBA.
                    let mut pixels = placement.image.rgba.as_ref().to_vec();
                    for pixel in pixels.chunks_exact_mut(4) {
                        pixel.swap(0, 2);
                    }
                    let frame: Option<image::RgbaImage> = image::ImageBuffer::from_raw(
                        placement.image.width,
                        placement.image.height,
                        pixels,
                    );
                    let frames = frame.map_or_else(smallvec::SmallVec::new, |buffer| {
                        smallvec::smallvec![image::Frame::new(buffer)]
                    });
                    std::sync::Arc::new(gpui::RenderImage::new(frames))
                });
            }
        }
        self.images
            .retain(|generation, _| live.contains(generation));
    }

    /// The application state the views read.
    pub(crate) fn app(&self) -> &Muxtrix {
        &self.app
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (app, effects) = Muxtrix::boot();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let inputs = crate::views::gpui::inputs::Inputs::new(window, cx);
        let mut root = Self {
            app,
            inputs,
            pending_focus: None,
            images: std::collections::BTreeMap::new(),
            focus,
        };
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
        self.sync_inputs(window, cx);
        window.set_window_title(&self.app.title());
        cx.notify();
    }

    /// Deliver a message from a context that has no `Window` — a timer or a
    /// channel. The window is reached through the entity's own handle.
    pub(crate) fn dispatch_detached(&mut self, message: Message, cx: &mut Context<Self>) {
        // A poll that found nothing new should not repaint. Everything else
        // might have changed something this cannot see, so it repaints.
        let quiet_poll = matches!(message, Message::PollTerminal);
        let before = quiet_poll.then(|| self.app.grid_revision());
        let effects = self.app.update(message);
        let repaint = before.is_none_or(|before| before != self.app.grid_revision());
        self.run_detached_effects(effects, cx);
        if repaint {
            self.sync_images();
            cx.notify();
        }
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
                // A real thread, not the background executor. This work shells
                // out to git and gh and blocks for as long as they take; on the
                // executor it occupies a pool thread, and the timers that share
                // that pool — cursor blink, the e2e tick — stop firing until it
                // returns.
                let (sender, receiver) = async_channel::bounded(1);
                std::thread::Builder::new()
                    .name("muxtrix-effect".into())
                    .spawn(move || {
                        let _ = sender.send_blocking(work());
                    })
                    .map_or_else(
                        |error| eprintln!("muxtrix: could not start background work: {error}"),
                        |_| {
                            cx.spawn(async move |this, cx| {
                                if let Ok(message) = receiver.recv().await {
                                    let _ = this.update(cx, |root, cx| {
                                        root.dispatch_detached(message, cx);
                                    });
                                }
                            })
                            .detach();
                        },
                    );
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
            // Focusing needs a window, and effects run without one — a timer
            // can produce them. Recorded here and applied on the next frame,
            // which is soon enough for a caret to appear.
            Effect::Focus(target) => self.pending_focus = Some(target),
            Effect::ScrollToRatio(..) | Effect::ScrollToOffset(..) => {}
            #[cfg(feature = "e2e")]
            Effect::ScrollToEnd(_) => {}
            #[cfg(feature = "e2e")]
            Effect::Capture => {
                // The frame is grabbed from outside this process; all that
                // happens here is asserting state and saying so on the control
                // socket, which `capture_ready` already reports. The window
                // deliberately stays up until the harness sends `Quit`.
                if let Err(error) = self.app.report_e2e_capture() {
                    eprintln!("muxtrix: e2e capture failed: {error}");
                    cx.quit();
                }
            }
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
fn repeat<F>(cx: &mut Context<Root>, every: Duration, mut message: F)
where
    F: FnMut(&Root) -> Option<Message> + 'static,
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
        // Focus follows what is open. An explicit request wins; otherwise the
        // root takes it back, because a field that keeps focus after its
        // surface closes swallows everything the terminal should receive.
        let target = self
            .pending_focus
            .take()
            .or_else(|| self.app.focus_target());
        match target.and_then(|target| self.inputs.get(target).cloned()) {
            Some(field) => {
                if !field.read(cx).focus_handle(cx).is_focused(window) {
                    field.update(cx, |state, cx| state.focus(window, cx));
                }
            }
            None => {
                if !self.focus.is_focused(window) {
                    self.focus.focus(window, cx);
                }
            }
        }
        let tokens = DesignTokens::for_appearance(self.app.settings.appearance);
        // Settings and the theme gallery replace the whole shell, as they do
        // under iced: they are screens, not panels.
        let screen = match self.app.active_view {
            crate::app::ActiveView::Settings | crate::app::ActiveView::ThemeGallery => {
                Some(self.view_settings(cx))
            }
            _ => None,
        };
        let sidebar = self.view_sidebar(cx);
        let github = self.github_panel(cx);
        let palette = self.command_palette(cx);
        let dialog = self.dialog(cx);
        let toast = self.toast();
        let status_bar = self.status_bar();
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
            .child(match screen {
                Some(screen) => div().flex_grow(1.0).overflow_hidden().child(screen),
                None => div()
                    .flex()
                    .flex_row()
                    .flex_grow(1.0)
                    .overflow_hidden()
                    .child(sidebar)
                    .child(div().flex_grow(1.0).overflow_hidden().child(workspace))
                    .children(github),
            })
            .children(status_bar)
            .children(menu)
            .children(dialog)
            .children(palette)
            .children(toast)
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
