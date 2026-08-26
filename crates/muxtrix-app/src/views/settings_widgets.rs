//! The stateful controls on the settings page, owned by the root.
//!
//! `gpui-component` pickers, sliders and text fields each carry an entity of
//! their own, so they cannot be made in `render`: they live here, are built
//! once with the window, and are kept in step with the settings draft in both
//! directions.
//!
//! Every picker is a list of display strings. The application's own enums
//! are mapped to and from those by position, which keeps one state type for
//! all seven pickers and leaves `Display` as the single source of their copy.

use gpui::{AppContext, Context, Entity, FocusHandle, Focusable, Window};
use gpui_component::IndexPath;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::select::{SelectEvent, SelectState};
use gpui_component::slider::{SliderEvent, SliderState, SliderValue};

use crate::app::{DefaultAgentChoice, Message};
use crate::runtime::gpui::Root;
use crate::settings::Appearance;
use crate::themes::TerminalThemeId;

/// A control paired with the message its change produces.
type BoundPicker<'a> = (&'a Picker, fn(&Root, usize) -> Option<Message>);
type BoundSlider<'a> = (&'a Entity<SliderState>, fn(f32) -> Message);
type BoundField<'a> = (&'a Entity<InputState>, fn(String) -> Message);

/// A picker over display strings, with the list it was last given.
///
/// The state does not expose its items, and replacing them on every frame
/// would reset the menu; remembering what it holds is what makes "only when
/// they differ" possible.
pub(crate) struct Picker {
    pub(crate) state: Entity<SelectState<Vec<String>>>,
    pub(crate) trigger_focus: FocusHandle,
    items: std::cell::RefCell<Vec<String>>,
}

/// Every control on the settings page that keeps state of its own.
pub(crate) struct SettingsWidgets {
    pub(crate) appearance: Picker,
    pub(crate) ui_font: Picker,
    pub(crate) ui_font_weight: Picker,
    pub(crate) terminal_theme: Picker,
    pub(crate) terminal_font: Picker,
    pub(crate) terminal_font_weight: Picker,
    pub(crate) default_agent: Picker,
    pub(crate) ui_font_size: Entity<SliderState>,
    pub(crate) terminal_font_size: Entity<SliderState>,
    pub(crate) terminal_line_height: Entity<SliderState>,
    pub(crate) scrollback: Entity<InputState>,
    pub(crate) github_host: Entity<InputState>,
    pub(crate) codex_command: Entity<InputState>,
    pub(crate) claude_command: Entity<InputState>,
    pub(crate) pi_command: Entity<InputState>,
}

impl SettingsWidgets {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Root>) -> Self {
        let widgets = Self {
            appearance: picker(window, cx),
            ui_font: picker(window, cx),
            ui_font_weight: picker(window, cx),
            terminal_theme: picker(window, cx),
            terminal_font: picker(window, cx),
            terminal_font_weight: picker(window, cx),
            default_agent: picker(window, cx),
            ui_font_size: slider(window, cx, 12.0, 20.0, 1.0),
            terminal_font_size: slider(window, cx, 10.0, 28.0, 1.0),
            terminal_line_height: slider(window, cx, 1.0, 1.6, 0.05),
            scrollback: field(window, cx, "10000"),
            github_host: field(window, cx, "github.com"),
            codex_command: field(window, cx, "codex"),
            claude_command: field(window, cx, "claude"),
            pi_command: field(window, cx, "omp"),
        };
        widgets.subscribe(cx);
        widgets
    }

    /// Turn each control's change into the message the application already
    /// understands. Pickers report an index, which is mapped back onto the
    /// choice list the root filled them from.
    fn subscribe(&self, cx: &mut Context<Root>) {
        let pickers: [BoundPicker<'_>; 7] = [
            (&self.appearance, |_, index| {
                Appearance::ALL
                    .get(index)
                    .copied()
                    .map(Message::SettingsAppearance)
            }),
            (&self.ui_font, |root, index| {
                root.app
                    .available_ui_fonts
                    .get(index)
                    .cloned()
                    .map(Message::SettingsUiFont)
            }),
            (&self.ui_font_weight, |root, index| {
                root.app
                    .available_ui_font_weights
                    .get(index)
                    .copied()
                    .map(Message::SettingsUiFontWeight)
            }),
            (&self.terminal_theme, |_, index| {
                TerminalThemeId::ALL
                    .get(index)
                    .copied()
                    .map(Message::SettingsTerminalTheme)
            }),
            (&self.terminal_font, |root, index| {
                root.app
                    .available_terminal_fonts
                    .get(index)
                    .cloned()
                    .map(Message::SettingsTerminalFont)
            }),
            (&self.terminal_font_weight, |root, index| {
                root.app
                    .available_terminal_font_weights
                    .get(index)
                    .copied()
                    .map(Message::SettingsTerminalFontWeight)
            }),
            (&self.default_agent, |root, index| {
                root.app
                    .default_agent_choices()
                    .get(index)
                    .copied()
                    .map(Message::SettingsDefaultAgent)
            }),
        ];
        for (picker, message) in pickers {
            cx.subscribe(
                &picker.state,
                move |root, picker, event: &SelectEvent<Vec<String>>, cx| {
                    let SelectEvent::Confirm(_) = event;
                    let Some(index) = picker.read(cx).selected_index(cx) else {
                        return;
                    };
                    if let Some(message) = message(root, index.row) {
                        root.dispatch_detached(message, cx);
                    }
                },
            )
            .detach();
        }

        let sliders: [BoundSlider<'_>; 3] = [
            (&self.ui_font_size, Message::SettingsUiFontSize),
            (&self.terminal_font_size, Message::SettingsTerminalFontSize),
            (&self.terminal_line_height, Message::SettingsLineHeight),
        ];
        for (slider, message) in sliders {
            cx.subscribe(slider, move |root, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    root.dispatch_detached(message(value.start()), cx);
                }
            })
            .detach();
        }

        let fields: [BoundField<'_>; 5] = [
            (&self.scrollback, Message::SettingsScrollbackLimit),
            (&self.github_host, Message::SettingsGitHubHost),
            (&self.codex_command, Message::SettingsCodexCommand),
            (&self.claude_command, Message::SettingsClaudeCommand),
            (&self.pi_command, Message::SettingsPiCommand),
        ];
        for (field, message) in fields {
            cx.subscribe(field, move |root, field, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = field.read(cx).value().to_string();
                    root.dispatch_detached(message(value), cx);
                }
            })
            .detach();
        }
    }
}

fn picker(window: &mut Window, cx: &mut Context<Root>) -> Picker {
    let state = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
    let trigger_focus = state.read(cx).focus_handle(cx);
    Picker {
        state,
        trigger_focus,
        items: std::cell::RefCell::new(Vec::new()),
    }
}

fn slider(
    window: &mut Window,
    cx: &mut Context<Root>,
    min: f32,
    max: f32,
    step: f32,
) -> Entity<SliderState> {
    let _ = window;
    cx.new(|_| SliderState::new().min(min).max(max).step(step))
}

fn field(
    window: &mut Window,
    cx: &mut Context<Root>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

impl Root {
    /// Push the settings draft back into the controls.
    ///
    /// Only where they differ: a picker re-told its own selection would close
    /// its menu, a slider re-set mid-drag would fight the pointer, and a field
    /// re-set mid-typing would move the caret to the end.
    pub(crate) fn sync_settings_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = &self.app.settings_draft;
        let appearance = Appearance::ALL
            .iter()
            .position(|choice| *choice == draft.appearance);
        let ui_font = self
            .app
            .available_ui_fonts
            .iter()
            .position(|choice| *choice == draft.ui_font);
        let ui_font_weight = self
            .app
            .available_ui_font_weights
            .iter()
            .position(|choice| *choice == draft.ui_font_weight);
        let terminal_theme = TerminalThemeId::ALL
            .iter()
            .position(|choice| *choice == draft.terminal_theme);
        let terminal_font = self
            .app
            .available_terminal_fonts
            .iter()
            .position(|choice| *choice == draft.terminal_font);
        let terminal_font_weight = self
            .app
            .available_terminal_font_weights
            .iter()
            .position(|choice| *choice == draft.terminal_font_weight);
        let default_agent_choices = self.app.default_agent_choices();
        let default_agent = default_agent_choices
            .iter()
            .position(|choice| *choice == self.app.default_agent_choice());

        let pickers: [(&Picker, Vec<String>, Option<usize>); 7] = [
            (
                &self.settings_widgets.appearance,
                Appearance::ALL.iter().map(ToString::to_string).collect(),
                appearance,
            ),
            (
                &self.settings_widgets.ui_font,
                self.app
                    .available_ui_fonts
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                ui_font,
            ),
            (
                &self.settings_widgets.ui_font_weight,
                self.app
                    .available_ui_font_weights
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                ui_font_weight,
            ),
            (
                &self.settings_widgets.terminal_theme,
                TerminalThemeId::ALL
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                terminal_theme,
            ),
            (
                &self.settings_widgets.terminal_font,
                self.app
                    .available_terminal_fonts
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                terminal_font,
            ),
            (
                &self.settings_widgets.terminal_font_weight,
                self.app
                    .available_terminal_font_weights
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                terminal_font_weight,
            ),
            (
                &self.settings_widgets.default_agent,
                default_agent_choices
                    .iter()
                    .map(DefaultAgentChoice::to_string)
                    .collect(),
                default_agent,
            ),
        ];
        for (picker, items, selected) in pickers {
            let state = picker.state.clone();
            let selected = selected.map(IndexPath::new);
            let items_changed = *picker.items.borrow() != items;
            let selection_changed = state.read(cx).selected_index(cx) != selected;
            if items_changed || selection_changed {
                if items_changed {
                    *picker.items.borrow_mut() = items.clone();
                }
                state.update(cx, |state, cx| {
                    if items_changed {
                        state.set_items(items, window, cx);
                    }
                    state.set_selected_index(selected, window, cx);
                });
            }
        }

        let sliders: [(&Entity<SliderState>, f32); 3] = [
            (&self.settings_widgets.ui_font_size, draft.ui_font_size),
            (
                &self.settings_widgets.terminal_font_size,
                draft.terminal_font_size,
            ),
            (
                &self.settings_widgets.terminal_line_height,
                draft.terminal_line_height,
            ),
        ];
        for (slider, value) in sliders {
            let slider = slider.clone();
            if (slider.read(cx).value().start() - value).abs() > f32::EPSILON {
                slider.update(cx, |state, cx| {
                    state.set_value(SliderValue::Single(value), window, cx);
                });
            }
        }

        let fields: [(&Entity<InputState>, String); 5] = [
            (
                &self.settings_widgets.scrollback,
                self.app.settings_scrollback_lines_input.clone(),
            ),
            (
                &self.settings_widgets.github_host,
                draft.github_host.clone(),
            ),
            (
                &self.settings_widgets.codex_command,
                draft.codex_command.clone(),
            ),
            (
                &self.settings_widgets.claude_command,
                draft.claude_command.clone(),
            ),
            (&self.settings_widgets.pi_command, draft.pi_command.clone()),
        ];
        for (field, value) in fields {
            let field = field.clone();
            if field.read(cx).value().as_ref() != value {
                field.update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
    }
}
