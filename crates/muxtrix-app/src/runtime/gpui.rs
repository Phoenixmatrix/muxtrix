//! The GPUI window and event-loop adapter.
//!
//! The application core remains framework-agnostic: `update` returns
//! [`Effect`] values and this module executes them against GPUI.

use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    ParentElement, Point, Render, Styled, Window, WindowBounds, WindowOptions, div, point, px,
    size,
};

use crate::app::{Message, Muxtrix};
use crate::effect::Effect;
use crate::input::{Key, KeyEvent, Named};
use crate::theme::DesignTokens;

/// How often the terminals are drained when nothing has woken them.
///
/// Wakeups coalesce through a one-slot channel, so a burst of output can
/// produce a single signal and the grid then sits unread until the next one.
/// Anything that reads the grid to decide what to do — whether a program has
/// asked for mouse reporting, where the cursor is — would be acting on a stale
/// answer. A poll that finds nothing new does not repaint, so this costs
/// almost nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(16);

/// How often the cursor blink phase flips.
const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// The GitHub panel's loading animation step.
const GITHUB_LOADING_INTERVAL: Duration = Duration::from_millis(90);
/// The e2e harness's scenario tick.
#[cfg(feature = "e2e")]
const E2E_INTERVAL: Duration = Duration::from_millis(50);

/// Start the application and run it until the window closes.
pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
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
/// runtime effects.
pub(crate) struct Root {
    pub(crate) app: Muxtrix,
    pub(crate) inputs: crate::views::inputs::Inputs,
    /// The settings page's pickers, sliders and fields.
    pub(crate) settings_widgets: crate::views::settings_widgets::SettingsWidgets,
    /// A focus request waiting for a frame to apply it against.
    pending_focus: Option<crate::effect::FocusTarget>,
    /// The title the window already carries, so an unchanged one costs
    /// nothing.
    title: String,
    /// The appearance `gpui-component`'s theme was last synced to.
    component_theme: Option<crate::settings::Appearance>,
    /// Where each pane card was painted last frame, so its menu can be placed
    /// against the card.
    pub(crate) pane_bounds: std::rc::Rc<
        std::cell::RefCell<std::collections::HashMap<muxtrix_domain::PaneId, Bounds<gpui::Pixels>>>,
    >,
    /// Inline terminal images, decoded once and kept while the emulator still
    /// references them. Keyed by the emulator's own generation, so an image
    /// that is redrawn unchanged is not decoded again.
    images: std::collections::BTreeMap<u64, std::sync::Arc<gpui::RenderImage>>,
    /// One handle per scrollable surface, so an effect that names a surface by
    /// role has something to move.
    pub(crate) scrolls: Scrolls,
    /// Scroll requests waiting for their surface to have an extent.
    ///
    /// An effect can arrive before the surface it names has ever been laid
    /// out — opening a panel and scrolling it are one gesture — and a handle
    /// with no content yet reports nowhere to go. Held until it does.
    pending_scrolls: Vec<(crate::effect::ScrollTarget, ScrollTo)>,
    /// The window's keyboard focus. Held by the root rather than by any
    /// surface: Muxtrix routes every key through one handler and decides there
    /// whether it belongs to a shortcut, a dialog or the terminal, and that
    /// decision has to keep working before the inputs of Phase 4 exist.
    focus: FocusHandle,
}

impl Root {
    /// Move any surface whose scroll request can now be honoured.
    ///
    /// A request survives until its surface has somewhere to go, because the
    /// alternative is silently dropping it on the frame the surface appeared.
    fn apply_pending_scrolls(&mut self) {
        self.pending_scrolls.retain(|(target, request)| {
            let handle = self.scrolls.get(*target);
            let travel = handle.max_offset().y.max(px(0.));
            if travel <= px(0.) {
                // Nothing to scroll yet. Either the surface has not been laid
                // out or it genuinely fits, and one more frame tells them apart.
                return true;
            }
            let offset = match request {
                ScrollTo::Ratio(ratio) => travel * ratio.clamp(0.0, 1.0),
                ScrollTo::Offset(offset) => px(*offset).min(travel),
                #[cfg(feature = "e2e")]
                ScrollTo::End => travel,
            };
            handle.set_offset(point(px(0.), -offset));
            false
        });
    }

    /// Tell the application where its scrollable surfaces have got to.
    ///
    /// GPUI scroll handles hold a position but announce nothing, and the
    /// GitHub panel's virtual lists need the offset to decide which rows to
    /// build. Reported only on a real change, so this settles rather than
    /// feeding itself.
    fn report_scrolls(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.app.github_panel.as_ref() else {
            return;
        };
        let files = -f32::from(self.scrolls.github_files.offset().y);
        let requests = -f32::from(self.scrolls.github_pull_requests.offset().y);
        // A chosen pull request's file list and the working tree's share one
        // handle and one message, but land in different fields — compare
        // against whichever the message will actually write, or this reports
        // the same move on every frame forever.
        let recorded = if panel.selected_pull_request_number.is_some() {
            panel.selected_pull_request_file_scroll_offset
        } else {
            panel.file_scroll_offset
        };
        let file_moved = (files - recorded).abs() > 0.5;
        let request_moved = (requests - panel.pull_request_scroll_offset).abs() > 0.5;
        // Deferred rather than dispatched here: this runs inside a render,
        // and updating state mid-frame means rendering against a value the
        // frame did not start with.
        let mut moves = Vec::new();
        if file_moved {
            moves.push(Message::GitHubFileScrolled(files.max(0.0)));
        }
        if request_moved {
            moves.push(Message::GitHubPullRequestScrolled(requests.max(0.0)));
        }
        if !moves.is_empty() {
            cx.spawn(async move |this, cx| {
                for message in moves {
                    let _ = this.update(cx, |root, cx| root.dispatch_detached(message, cx));
                }
            })
            .detach();
        }
    }

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
        let inputs = crate::views::inputs::Inputs::new(window, cx);
        let settings_widgets = crate::views::settings_widgets::SettingsWidgets::new(window, cx);
        let mut root = Self {
            app,
            inputs,
            settings_widgets,
            pending_focus: None,
            title: String::new(),
            component_theme: None,
            pane_bounds: std::rc::Rc::default(),
            images: std::collections::BTreeMap::new(),
            scrolls: Scrolls::default(),
            pending_scrolls: Vec::new(),
            focus,
        };
        root.spawn_timers(cx);
        root.spawn_terminal_wakeups(cx);
        root.observe_window(window, cx);
        root.run_effects(effects, window, cx);
        // The startup terminal is launched by `WindowOpened`. There is a
        // window by the time this runs, so report its real bounds directly.
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
        // What a key or a click means can depend on terminal state the PTY
        // has already reported but this process has not read yet — whether
        // the program is in mouse-reporting mode, say. Draining first means
        // the decision is made against what is true now rather than against
        // whatever the last frame happened to see.
        self.drain_terminals(cx);
        // A key the terminal owns changes nothing this process draws: the
        // program echoes, and the echo arrives as terminal output that
        // repaints on its own. Drawing for the keystroke as well caps input
        // at one key per frame, because the draw happens inline before the
        // next X event is read — enough to fall seconds behind a paste or an
        // automated run.
        // Pointer motion arrives many times a frame and changes nothing to
        // draw unless a drag is under way; it is fingerprinted like a key.
        let quiet = (matches!(message, Message::Keyboard(KeyEvent::Pressed(_)))
            && self.app.focus_target().is_none())
            || (matches!(message, Message::PointerMoved(_))
                && self.app.split_drag.is_none()
                && self.app.tab_drag.is_none()
                && self.app.terminal_scroll_drag.is_none());
        let before = quiet.then(|| (self.app.grid_revision(), self.app.chrome_revision()));
        let effects = self.app.update(message);
        let repaint = before
            .is_none_or(|before| before != (self.app.grid_revision(), self.app.chrome_revision()));
        self.run_effects(effects, window, cx);
        self.sync_inputs(window, cx);
        self.sync_settings_widgets(window, cx);
        // Setting the title is a round trip to the X server. The title
        // changes when a pane's does; a keystroke is not that, and paying for
        // one per key is enough to fall behind a fast typist.
        let title = self.app.title();
        if self.title != title {
            window.set_window_title(&title);
            self.title = title;
        }
        if repaint {
            cx.notify();
        }
    }

    /// Drain the terminals once per frame.
    ///
    /// The X11 backend drives a redraw every display refresh whether or not
    /// anything is dirty, and it services that timer from the same event loop
    /// that runs spawned tasks. Under a software renderer a frame can outlast
    /// the interval, and the executor then goes seconds without running a
    /// task — long enough for the queue to coalesce away the frame that
    /// carried a mode change, which is how a program that had turned mouse
    /// reporting on could be missed entirely.
    ///
    /// The frame itself is the one clock that cannot be starved by drawing, so
    /// the drain hangs off it. [`POLL_INTERVAL`] stays as the fallback for
    /// when there are no frames at all: a hidden or unmapped window stops the
    /// refresh loop, and a terminal still has to be read.
    pub(crate) fn drain_terminals(&mut self, cx: &mut Context<Self>) {
        let effects = self.app.update(Message::PollTerminal);
        self.run_detached_effects(effects, cx);
        self.sync_images();
        #[cfg(feature = "e2e")]
        self.app.observe_e2e();
    }

    /// Deliver a message from a context that has no `Window` — a timer or a
    /// channel. The window is reached through the entity's own handle.
    pub(crate) fn dispatch_detached(&mut self, message: Message, cx: &mut Context<Self>) {
        // A tick that changed nothing should not repaint. Polls and the e2e
        // clock both fire many times a second and usually change nothing;
        // under a software renderer, repainting for each of them starves the
        // very subprocesses this app exists to host. Everything else might
        // have changed something these fingerprints cannot see, so it
        // repaints unconditionally.
        let quiet = matches!(message, Message::PollTerminal | Message::BlinkCursor);
        #[cfg(feature = "e2e")]
        let quiet = quiet || matches!(message, Message::E2eTick);
        let before = quiet.then(|| (self.app.grid_revision(), self.app.chrome_revision()));
        let effects = self.app.update(message);
        let repaint = before
            .is_none_or(|before| before != (self.app.grid_revision(), self.app.chrome_revision()));
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
            // Focusing needs a window, and effects run without one — a timer
            // can produce them. Recorded here and applied on the next frame,
            // which is soon enough for a caret to appear.
            Effect::Focus(target) => self.pending_focus = Some(target),
            // Held rather than applied: an effect can name a surface that has
            // not been laid out yet — opening a panel and scrolling it are one
            // gesture — and a handle with no content reports nowhere to go.
            Effect::ScrollToRatio(target, ratio) => {
                self.pending_scrolls.push((target, ScrollTo::Ratio(ratio)));
            }
            Effect::ScrollToOffset(target, offset) => {
                self.pending_scrolls
                    .push((target, ScrollTo::Offset(offset)));
            }
            #[cfg(feature = "e2e")]
            Effect::ScrollToEnd(target) => {
                self.pending_scrolls.push((target, ScrollTo::End));
            }
            #[cfg(feature = "e2e")]
            Effect::Capture => {
                // The scenario staged a state for this frame to show. Whatever
                // the tick's fingerprint thought, this is the one frame that
                // has to be drawn before anyone looks.
                cx.notify();
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
    /// Starts independent loops for periodic application work.
    ///
    /// Separate loops keep each cadence and its gating conditions local, so a
    /// paused animation costs no unrelated polling work.
    fn spawn_timers(&mut self, cx: &mut Context<Self>) {
        repeat(cx, POLL_INTERVAL, |root| {
            (!root.app.terminals.is_empty()).then_some(Message::PollTerminal)
        });
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
    /// one poll.
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

    /// Settings widgets own their editing keys. Sending the same key through
    /// the application reducer synchronizes the old draft back into the widget
    /// before its `InputEvent::Change` subscription can publish the edit.
    fn settings_component_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.app.active_view == crate::app::ActiveView::Settings
            && self.focus.contains_focused(window, cx)
            && !self.focus.is_focused(window)
    }

    /// Every key the window receives, in the app's own vocabulary.
    /// The terminal receives every key the application does not explicitly
    /// claim; this adapter does not maintain a second keymap.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let input = crate::input::from_keystroke(&event.keystroke);
        if self.settings_component_focused(window, cx)
            && !matches!(input.modified_key.as_ref(), Key::Named(Named::Escape))
        {
            return;
        }
        self.dispatch(Message::Keyboard(KeyEvent::Pressed(input)), window, cx);
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_component_focused(window, cx) {
            return;
        }
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
        if self.settings_component_focused(window, cx) {
            return;
        }
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
        self.drain_terminals(cx);
        self.sync_component_theme(window, cx);
        // Both of these read what the last frame laid out, so they belong at
        // the start of the next one.
        self.apply_pending_scrolls();
        self.report_scrolls(cx);
        // Focus follows what is open. An explicit request wins; otherwise the
        // root takes it back, because a field that keeps focus after its
        // surface closes swallows everything the terminal should receive.
        let target = self
            .pending_focus
            .take()
            .or_else(|| self.app.focus_target());
        match target {
            Some(target) => match self.inputs.get(target).cloned() {
                Some(field) => {
                    if !field.read(cx).focus_handle(cx).is_focused(window) {
                        let select_all = matches!(
                            target,
                            crate::effect::FocusTarget::WorkspaceCreate
                                | crate::effect::FocusTarget::Rename
                        );
                        field.update(cx, |state, cx| {
                            state.focus(window, cx);
                            if select_all {
                                state.select_all(window, cx);
                            }
                        });
                    }
                }
                None => {
                    if !self.focus.is_focused(window) {
                        self.focus.focus(window, cx);
                    }
                }
            },
            None => {
                let settings_control_focused = self.app.active_view
                    == crate::app::ActiveView::Settings
                    && self.focus.contains_focused(window, cx);
                if !settings_control_focused && !self.focus.is_focused(window) {
                    self.focus.focus(window, cx);
                }
            }
        }
        let tokens = DesignTokens::for_appearance(self.app.settings.appearance);
        // Settings and the theme gallery replace the whole shell rather than
        // behaving as panels.
        let screen = match self.app.active_view {
            crate::app::ActiveView::Settings | crate::app::ActiveView::ThemeGallery => {
                Some(self.view_settings(cx))
            }
            // The diff keeps the GitHub panel docked beside it and drops the
            // sidebar: the file list is what you navigate from here.
            crate::app::ActiveView::GitHubDiff => Some(
                div()
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(
                        div()
                            .flex_grow(1.0)
                            .min_w(px(0.))
                            .overflow_hidden()
                            .child(self.github_diff_view(tokens, cx)),
                    )
                    .children(self.github_panel(cx))
                    .into_any_element(),
            ),
            crate::app::ActiveView::Workspace => None,
        };
        let sidebar = self.view_sidebar(cx);
        let github = self.github_panel(cx);
        let palette = self.command_palette(cx);
        let dialog = self.dialog(cx);
        let toast = self.toast();
        let status_bar = self.status_bar();
        let workspace = self.view_workspace(window, cx);
        // The menu floats above the shell. A press outside dismisses it and is
        // consumed, as `pane_menu_click_away_observed` asserts.
        let menu = self.pane_menu(cx).map(|menu| {
            // Keep the menu six pixels in from the card's right edge and 38
            // pixels below its top, constrained to the window.
            let anchor = self
                .app
                .pane_menu
                .and_then(|pane_id| self.pane_bounds.borrow().get(&pane_id).copied())
                .map_or_else(
                    || root_menu_anchor(window),
                    |bounds| point(bounds.right() - px(6.), bounds.top() + px(38.)),
                );
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
                            .position(anchor)
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
            // Window-level pointer tracking: a split handle or a tab that was
            // grabbed keeps following the pointer wherever it goes, and a
            // release anywhere ends whatever was in progress.
            .on_mouse_move(
                cx.listener(|root, event: &gpui::MouseMoveEvent, window, cx| {
                    let position = crate::geom::Point::new(
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                    );
                    root.dispatch(Message::PointerMoved(position), window, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|root, _: &gpui::MouseUpEvent, window, cx| {
                    root.dispatch(Message::EndPointerInteraction, window, cx);
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(color(tokens.app))
            // Name the resolved system face explicitly; an unnamed GPUI
            // fallback has different metrics and changes chrome width.
            .font_family(ui_family(&self.app.settings))
            .font_weight(gpui::FontWeight(f32::from(
                self.app.settings.ui_font_weight.numeric(),
            )))
            .child(match screen {
                Some(screen) => div().flex_grow(1.0).overflow_hidden().child(screen),
                // The panel docks beside the workspace when there is room
                // and floats over its right edge when there is not; the
                // panel positions itself for the second case, so the row
                // only has to be relative for it to float against.
                None => div()
                    .relative()
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
impl Root {
    /// Keep `gpui-component`'s own theme on the app's design tokens.
    ///
    /// The stock widgets — pickers, switches, sliders, text fields — read the
    /// library's global theme, not the tokens the rest of the chrome is drawn
    /// from. Left alone they render in the library's light default, white on
    /// a dark page. Mapping the tokens over it, whenever the appearance the
    /// page is drawn in changes, is what makes a `Select` sit on a settings
    /// row as if it were drawn there by hand.
    fn sync_component_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_component::{Theme, ThemeMode};
        let appearance = self.app.settings_draft.appearance;
        if self.component_theme == Some(appearance) {
            return;
        }
        self.component_theme = Some(appearance);
        let tokens = DesignTokens::for_appearance(appearance);
        let mode = match appearance {
            crate::settings::Appearance::Light => ThemeMode::Light,
            crate::settings::Appearance::System | crate::settings::Appearance::Dark => {
                ThemeMode::Dark
            }
        };
        Theme::change(mode, None, cx);
        let hsla = |value: crate::theme::Color| -> gpui::Hsla { color(value).into() };
        let faded = |value: crate::theme::Color, alpha: f32| -> gpui::Hsla {
            let mut faded = color(value);
            faded.a = alpha;
            faded.into()
        };
        let theme = Theme::global_mut(cx);
        theme.font_family = ui_family(&self.app.settings);
        theme.font_size = px(self.app.settings.ui_pixels(11.0));
        theme.mono_font_family = crate::views::terminal_family(&self.app.settings);
        theme.radius = px(5.);
        theme.radius_lg = px(7.);
        theme.shadow = false;
        let colors = &mut theme.colors;
        colors.background = hsla(tokens.app);
        colors.foreground = hsla(tokens.text);
        colors.border = hsla(tokens.line_strong);
        colors.input = hsla(tokens.line_strong);
        colors.ring = faded(tokens.accent, 0.6);
        colors.caret = hsla(tokens.text);
        colors.selection = faded(tokens.accent, 0.35);
        colors.muted = hsla(tokens.panel_raised);
        colors.muted_foreground = hsla(tokens.muted);
        colors.accent = hsla(tokens.panel_raised);
        colors.accent_foreground = hsla(tokens.text);
        colors.primary = hsla(tokens.accent);
        colors.primary_hover = faded(tokens.accent, 0.86);
        colors.primary_active = faded(tokens.accent, 0.8);
        colors.primary_foreground = hsla(tokens.app);
        colors.secondary = hsla(tokens.panel);
        colors.secondary_hover = hsla(tokens.panel_raised);
        colors.secondary_active = hsla(tokens.panel_raised);
        colors.secondary_foreground = hsla(tokens.text);
        colors.popover = hsla(tokens.overlay);
        colors.popover_foreground = hsla(tokens.text);
        colors.list = hsla(tokens.overlay);
        colors.list_hover = hsla(tokens.panel_raised);
        colors.list_active = faded(tokens.accent, 0.18);
        colors.list_active_border = hsla(tokens.accent);
        colors.list_even = hsla(tokens.overlay);
        colors.list_head = hsla(tokens.overlay);
        colors.slider_bar = hsla(tokens.accent);
        colors.slider_thumb = hsla(tokens.accent);
        colors.switch = hsla(tokens.line_strong);
        colors.switch_thumb = hsla(tokens.text);
        colors.scrollbar = hsla(crate::theme::Color::TRANSPARENT);
        colors.scrollbar_thumb = faded(tokens.text, 0.25);
        colors.scrollbar_thumb_hover = faded(tokens.text, 0.4);
        colors.danger = hsla(tokens.danger);
        colors.danger_foreground = hsla(tokens.app);
        colors.success = hsla(tokens.success);
        colors.success_foreground = hsla(tokens.app);
        colors.warning = hsla(tokens.warning);
        colors.warning_foreground = hsla(tokens.app);
        colors.link = hsla(tokens.accent);
        colors.link_hover = hsla(tokens.accent);
        colors.link_active = hsla(tokens.accent);
        colors.overlay = hsla(tokens.scrim);
        window.refresh();
    }
}

/// The chrome's face: the configured UI font, or the platform's sans-serif.
pub(crate) fn ui_family(settings: &crate::settings::AppSettings) -> gpui::SharedString {
    settings
        .ui_font
        .family_name()
        .map_or_else(
            || {
                crate::metrics::system_sans_family()
                    .unwrap_or("sans-serif")
                    .to_owned()
            },
            ToOwned::to_owned,
        )
        .into()
}

pub(crate) fn color(value: crate::theme::Color) -> gpui::Rgba {
    gpui::Rgba {
        r: value.r,
        g: value.g,
        b: value.b,
        a: value.a,
    }
}

/// Fallback anchor when a pane card has not been painted yet.
///
/// GPUI elements report their painted bounds after rendering, so the first
/// frame uses the window's top-right where the header action lives.
fn root_menu_anchor(window: &Window) -> Point<gpui::Pixels> {
    let bounds = window.bounds();
    point(bounds.size.width - px(6.), px(38.))
}

/// Every scrollable surface in the shell, addressed the way effects name them.
#[derive(Default)]
pub(crate) struct Scrolls {
    pub(crate) settings: gpui::ScrollHandle,
    pub(crate) palette: gpui::ScrollHandle,
    pub(crate) github_files: gpui::ScrollHandle,
    pub(crate) github_pull_requests: gpui::ScrollHandle,
}

impl Scrolls {
    fn get(&self, target: crate::effect::ScrollTarget) -> &gpui::ScrollHandle {
        use crate::effect::ScrollTarget;
        match target {
            ScrollTarget::Settings => &self.settings,
            ScrollTarget::CommandPalette => &self.palette,
            ScrollTarget::GitHubFiles => &self.github_files,
            ScrollTarget::GitHubPullRequests => &self.github_pull_requests,
        }
    }
}

/// Where a scroll request wants its surface to end up.
enum ScrollTo {
    Ratio(f32),
    Offset(f32),
    /// All the way to the end, whatever the extent turns out to be. Only the
    /// e2e scenarios ask for it.
    #[cfg(feature = "e2e")]
    End,
}
