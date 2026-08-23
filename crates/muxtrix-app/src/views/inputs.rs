//! The text fields the shell edits, one per [`FocusTarget`].
//!
//! Editing needs real inputs: a field that only displays a draft cannot show a
//! caret, a selection, or respond to Home. These are `gpui-component`
//! [`InputState`]s owned by the root, kept in step with application drafts in
//! both directions: edits emit the same application messages as every other
//! interaction, while commands and restored sessions can update the fields.

use gpui::{AppContext, Context, Entity, Window};
use gpui_component::input::{InputEvent, InputState};

use crate::app::Message;
use crate::effect::FocusTarget;
use crate::runtime::gpui::Root;

/// A field paired with the message its edits produce.
type Bound<'a> = (&'a Entity<InputState>, fn(String) -> Message);

/// Every editable field in the shell.
pub(crate) struct Inputs {
    pub(crate) palette: Entity<InputState>,
    pub(crate) workspace_create: Entity<InputState>,
    pub(crate) rename: Entity<InputState>,
    pub(crate) worktree: Entity<InputState>,
    pub(crate) github_query: Entity<InputState>,
}

impl Inputs {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Root>) -> Self {
        let inputs = Self {
            palette: field(window, cx, "Type a command…"),
            workspace_create: field(window, cx, "Workspace name"),
            rename: field(window, cx, "Name"),
            worktree: field(window, cx, "Worktree name"),
            github_query: field(window, cx, "Filter pull requests"),
        };
        inputs.subscribe(cx);
        inputs
    }

    pub(crate) fn get(&self, target: FocusTarget) -> Option<&Entity<InputState>> {
        match target {
            FocusTarget::CommandPalette => Some(&self.palette),
            FocusTarget::WorkspaceCreate => Some(&self.workspace_create),
            FocusTarget::Rename => Some(&self.rename),
            FocusTarget::Worktree => Some(&self.worktree),
            FocusTarget::GitHubPullRequestQuery => Some(&self.github_query),
            // Not a real widget: focusing it means "take focus off the search
            // field", which under GPUI is done by focusing the root instead.
            FocusTarget::GitHubKeyboardSink => None,
        }
    }

    /// Turn edits into the messages the application already understands.
    fn subscribe(&self, cx: &mut Context<Root>) {
        let fields: [Bound<'_>; 5] = [
            (&self.palette, Message::CommandQueryChanged),
            (&self.workspace_create, Message::WorkspaceNameChanged),
            (&self.rename, Message::RenameDraftChanged),
            (&self.worktree, Message::WorktreeNameChanged),
            (&self.github_query, Message::GitHubPullRequestQueryChanged),
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

fn field(
    window: &mut Window,
    cx: &mut Context<Root>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

impl Root {
    /// Push application drafts back into the fields.
    ///
    /// Only when they actually differ: writing a value a field already holds
    /// would move the caret to the end mid-typing.
    pub(crate) fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pairs: [(&Entity<InputState>, String); 5] = [
            (&self.inputs.palette, self.app.palette.query.clone()),
            (
                &self.inputs.workspace_create,
                self.app.workspace_name_draft.clone(),
            ),
            (&self.inputs.rename, self.app.rename_draft.clone()),
            (&self.inputs.worktree, self.app.worktree_name_draft.clone()),
            (
                &self.inputs.github_query,
                self.app
                    .github_panel
                    .as_ref()
                    .map(|panel| panel.pull_request_query.clone())
                    .unwrap_or_default(),
            ),
        ];
        for (field, value) in pairs {
            let field = field.clone();
            if field.read(cx).value().as_ref() != value {
                field.update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
    }
}
