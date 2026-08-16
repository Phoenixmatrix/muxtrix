//! Serializable application state without UI or process handles.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 3;

macro_rules! entity_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

entity_id!(WorkspaceId);
entity_id!(TabId);
entity_id!(PaneId);
entity_id!(SurfaceId);
entity_id!(ProfileId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct SplitRatio(u16);

impl SplitRatio {
    pub const MIN: u16 = 100;
    pub const MAX: u16 = 900;
    pub const EQUAL: Self = Self(500);

    pub fn new(permille: u16) -> Result<Self, DomainError> {
        if (Self::MIN..=Self::MAX).contains(&permille) {
            Ok(Self(permille))
        } else {
            Err(DomainError::InvalidSplitRatio(permille))
        }
    }

    #[must_use]
    pub const fn permille(self) -> u16 {
        self.0
    }

    #[must_use]
    pub fn fraction(self) -> f32 {
        f32::from(self.0) / 1_000.0
    }
}

impl Default for SplitRatio {
    fn default() -> Self {
        Self::EQUAL
    }
}

impl TryFrom<u16> for SplitRatio {
    type Error = DomainError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SplitRatio> for u16 {
    fn from(value: SplitRatio) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum PaneTree {
    Leaf {
        pane_id: PaneId,
    },
    /// Panes sharing one footprint. The focused pane is expanded by the UI;
    /// every other pane remains available as a title-height sheet.
    Stack {
        pane_ids: Vec<PaneId>,
    },
    Split {
        axis: SplitAxis,
        ratio: SplitRatio,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl PaneTree {
    #[must_use]
    pub const fn leaf(pane_id: PaneId) -> Self {
        Self::Leaf { pane_id }
    }

    #[must_use]
    pub fn stack(pane_ids: Vec<PaneId>) -> Option<Self> {
        match pane_ids.as_slice() {
            [] => None,
            [pane_id] => Some(Self::leaf(*pane_id)),
            _ => Some(Self::Stack { pane_ids }),
        }
    }

    #[must_use]
    pub fn contains(&self, target: PaneId) -> bool {
        match self {
            Self::Leaf { pane_id } => *pane_id == target,
            Self::Stack { pane_ids } => pane_ids.contains(&target),
            Self::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    #[must_use]
    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        self.collect_pane_ids(&mut ids);
        ids
    }

    fn collect_pane_ids(&self, ids: &mut Vec<PaneId>) {
        match self {
            Self::Leaf { pane_id } => ids.push(*pane_id),
            Self::Stack { pane_ids } => ids.extend(pane_ids),
            Self::Split { first, second, .. } => {
                first.collect_pane_ids(ids);
                second.collect_pane_ids(ids);
            }
        }
    }

    fn split(
        &mut self,
        target: PaneId,
        new_pane: PaneId,
        axis: SplitAxis,
        ratio: SplitRatio,
    ) -> bool {
        match self {
            Self::Leaf { pane_id } if *pane_id == target => {
                let old_pane = *pane_id;
                *self = Self::Split {
                    axis,
                    ratio,
                    first: Box::new(Self::leaf(old_pane)),
                    second: Box::new(Self::leaf(new_pane)),
                };
                true
            }
            Self::Leaf { .. } => false,
            Self::Stack { pane_ids } if pane_ids.contains(&target) => {
                let old_stack = std::mem::replace(self, Self::leaf(new_pane));
                *self = Self::Split {
                    axis,
                    ratio,
                    first: Box::new(old_stack),
                    second: Box::new(Self::leaf(new_pane)),
                };
                true
            }
            Self::Stack { .. } => false,
            Self::Split { first, second, .. } => {
                first.split(target, new_pane, axis, ratio)
                    || second.split(target, new_pane, axis, ratio)
            }
        }
    }

    fn without(self, target: PaneId) -> (Option<Self>, bool) {
        match self {
            Self::Leaf { pane_id } if pane_id == target => (None, true),
            leaf @ Self::Leaf { .. } => (Some(leaf), false),
            Self::Stack { mut pane_ids } => {
                let Some(index) = pane_ids.iter().position(|pane_id| *pane_id == target) else {
                    return (Some(Self::Stack { pane_ids }), false);
                };
                pane_ids.remove(index);
                (Self::stack(pane_ids), true)
            }
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first, removed) = first.without(target);
                if removed {
                    let tree = match first {
                        Some(first) => Self::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second,
                        },
                        None => *second,
                    };
                    return (Some(tree), true);
                }

                let first = first.expect("an unchanged subtree is always present");
                let (second, removed) = second.without(target);
                if removed {
                    let tree = match second {
                        Some(second) => Self::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second: Box::new(second),
                        },
                        None => first,
                    };
                    (Some(tree), true)
                } else {
                    let second = second.expect("an unchanged subtree is always present");
                    (
                        Some(Self::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second: Box::new(second),
                        }),
                        false,
                    )
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pane {
    pub id: PaneId,
    #[serde(alias = "tabs")]
    pub surfaces: Vec<Surface>,
    pub active_surface_id: SurfaceId,
    pub attention: AttentionState,
    /// A user-chosen name that overrides surface and agent titles until cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    /// The interactive agent hosted by this pane, when known. This durable
    /// identity lets a new Muxtrix instance reclassify the replayed live screen
    /// without waiting for a lifecycle hook from the already-running process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<PaneAgent>,
}

impl Pane {
    #[must_use]
    pub fn new(surface: Surface) -> Self {
        Self {
            id: PaneId::new(),
            active_surface_id: surface.id,
            surfaces: vec![surface],
            attention: AttentionState::default(),
            custom_name: None,
            agent: None,
        }
    }

    #[must_use]
    pub fn active_surface(&self) -> Option<&Surface> {
        self.surfaces
            .iter()
            .find(|surface| surface.id == self.active_surface_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaneAgent {
    Codex,
    ClaudeCode,
    OhMyPi,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionState {
    pub unread_count: u32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Surface {
    pub id: SurfaceId,
    pub title: String,
    pub kind: SurfaceKind,
}

impl Surface {
    #[must_use]
    pub fn terminal(title: impl Into<String>, terminal: TerminalSurface) -> Self {
        Self {
            id: SurfaceId::new(),
            title: title.into(),
            kind: SurfaceKind::Terminal(terminal),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SurfaceKind {
    Terminal(TerminalSurface),
    Browser { url: String },
    Markdown { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSurface {
    pub profile_id: ProfileId,
    pub working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProfile {
    pub id: ProfileId,
    pub name: String,
    pub backend: ProcessBackend,
    pub program: String,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessBackend {
    Local,
    Wsl { distribution: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTab {
    pub id: TabId,
    pub name: String,
    pub root: PaneTree,
    pub panes: BTreeMap<PaneId, Pane>,
    pub focused_pane_id: PaneId,
}

impl WorkspaceTab {
    #[must_use]
    pub fn new(name: impl Into<String>, initial_surface: Surface) -> Self {
        let pane = Pane::new(initial_surface);
        let pane_id = pane.id;
        Self {
            id: TabId::new(),
            name: name.into(),
            root: PaneTree::leaf(pane_id),
            panes: BTreeMap::from([(pane_id, pane)]),
            focused_pane_id: pane_id,
        }
    }

    pub fn split_focused(
        &mut self,
        axis: SplitAxis,
        ratio: SplitRatio,
        surface: Surface,
    ) -> Result<PaneId, DomainError> {
        let pane = Pane::new(surface);
        let pane_id = pane.id;
        if !self.root.split(self.focused_pane_id, pane_id, axis, ratio) {
            return Err(DomainError::PaneNotFound(self.focused_pane_id));
        }
        self.panes.insert(pane_id, pane);
        self.focused_pane_id = pane_id;
        Ok(pane_id)
    }

    pub fn close_pane(&mut self, pane_id: PaneId) -> Result<(), DomainError> {
        if self.panes.len() == 1 {
            return Err(DomainError::CannotCloseLastPane);
        }
        if !self.panes.contains_key(&pane_id) {
            return Err(DomainError::PaneNotFound(pane_id));
        }

        let next_focus = self
            .root
            .pane_ids()
            .into_iter()
            .find(|candidate| *candidate != pane_id)
            .ok_or(DomainError::CannotCloseLastPane)?;
        let (root, removed) = self.root.clone().without(pane_id);
        if !removed {
            return Err(DomainError::PaneNotFound(pane_id));
        }
        self.root = root.ok_or(DomainError::CannotCloseLastPane)?;
        self.panes.remove(&pane_id);
        if self.focused_pane_id == pane_id {
            self.focused_pane_id = next_focus;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        let tree_ids = self.root.pane_ids();
        if tree_ids.len() != self.panes.len()
            || tree_ids
                .iter()
                .any(|pane_id| !self.panes.contains_key(pane_id))
        {
            return Err(DomainError::InconsistentPaneTree);
        }
        if !self.panes.contains_key(&self.focused_pane_id) {
            return Err(DomainError::PaneNotFound(self.focused_pane_id));
        }
        for pane in self.panes.values() {
            if pane.surfaces.is_empty()
                || !pane
                    .surfaces
                    .iter()
                    .any(|surface| surface.id == pane.active_surface_id)
            {
                return Err(DomainError::InvalidActiveSurface(pane.id));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub tabs: Vec<WorkspaceTab>,
    pub active_tab_id: TabId,
}

impl Workspace {
    #[must_use]
    pub fn new(name: impl Into<String>, initial_surface: Surface) -> Self {
        let tab = WorkspaceTab::new("Tab 1", initial_surface);
        Self {
            id: WorkspaceId::new(),
            name: name.into(),
            active_tab_id: tab.id,
            tabs: vec![tab],
        }
    }

    #[must_use]
    pub fn active_tab(&self) -> Option<&WorkspaceTab> {
        self.tabs.iter().find(|tab| tab.id == self.active_tab_id)
    }

    #[must_use]
    pub fn active_tab_mut(&mut self) -> Option<&mut WorkspaceTab> {
        self.tabs
            .iter_mut()
            .find(|tab| tab.id == self.active_tab_id)
    }

    #[must_use]
    pub fn tab(&self, tab_id: TabId) -> Option<&WorkspaceTab> {
        self.tabs.iter().find(|tab| tab.id == tab_id)
    }

    #[must_use]
    pub fn tab_mut(&mut self, tab_id: TabId) -> Option<&mut WorkspaceTab> {
        self.tabs.iter_mut().find(|tab| tab.id == tab_id)
    }

    #[must_use]
    pub fn pane(&self, pane_id: PaneId) -> Option<&Pane> {
        self.tabs.iter().find_map(|tab| tab.panes.get(&pane_id))
    }

    #[must_use]
    pub fn pane_mut(&mut self, pane_id: PaneId) -> Option<&mut Pane> {
        self.tabs
            .iter_mut()
            .find_map(|tab| tab.panes.get_mut(&pane_id))
    }

    #[must_use]
    pub fn tab_containing_pane(&self, pane_id: PaneId) -> Option<&WorkspaceTab> {
        self.tabs
            .iter()
            .find(|tab| tab.panes.contains_key(&pane_id))
    }

    #[must_use]
    pub fn all_pane_ids(&self) -> Vec<PaneId> {
        self.tabs
            .iter()
            .flat_map(|tab| tab.root.pane_ids())
            .collect()
    }

    #[must_use]
    pub fn pane_count(&self) -> usize {
        self.tabs.iter().map(|tab| tab.panes.len()).sum()
    }

    pub fn add_tab(&mut self, tab: WorkspaceTab) -> Result<(), DomainError> {
        if self.tabs.iter().any(|candidate| candidate.id == tab.id) {
            return Err(DomainError::DuplicateTab(tab.id));
        }
        self.active_tab_id = tab.id;
        self.tabs.push(tab);
        Ok(())
    }

    pub fn switch_tab(&mut self, tab_id: TabId) -> Result<(), DomainError> {
        if !self.tabs.iter().any(|tab| tab.id == tab_id) {
            return Err(DomainError::TabNotFound(tab_id));
        }
        self.active_tab_id = tab_id;
        Ok(())
    }

    pub fn rename_tab(&mut self, tab_id: TabId, name: impl AsRef<str>) -> Result<(), DomainError> {
        let name = name.as_ref().trim();
        if name.is_empty() {
            return Err(DomainError::InvalidTabName);
        }
        self.tab_mut(tab_id)
            .ok_or(DomainError::TabNotFound(tab_id))?
            .name = name.to_owned();
        Ok(())
    }

    pub fn close_tab(&mut self, tab_id: TabId) -> Result<WorkspaceTab, DomainError> {
        if self.tabs.len() == 1 {
            return Err(DomainError::CannotCloseLastTab);
        }
        let index = self
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .ok_or(DomainError::TabNotFound(tab_id))?;
        let removed = self.tabs.remove(index);
        if self.active_tab_id == tab_id {
            let next_index = index.min(self.tabs.len() - 1);
            self.active_tab_id = self.tabs[next_index].id;
        }
        Ok(removed)
    }

    pub fn split_focused(
        &mut self,
        axis: SplitAxis,
        ratio: SplitRatio,
        surface: Surface,
    ) -> Result<PaneId, DomainError> {
        let active_tab_id = self.active_tab_id;
        self.active_tab_mut()
            .ok_or(DomainError::TabNotFound(active_tab_id))?
            .split_focused(axis, ratio, surface)
    }

    pub fn close_pane(&mut self, pane_id: PaneId) -> Result<(), DomainError> {
        self.tabs
            .iter_mut()
            .find(|tab| tab.panes.contains_key(&pane_id))
            .ok_or(DomainError::PaneNotFound(pane_id))?
            .close_pane(pane_id)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.tabs.is_empty() {
            return Err(DomainError::WorkspaceHasNoTabs(self.id));
        }
        if !self.tabs.iter().any(|tab| tab.id == self.active_tab_id) {
            return Err(DomainError::TabNotFound(self.active_tab_id));
        }
        let mut tab_ids = std::collections::BTreeSet::new();
        let mut pane_ids = std::collections::BTreeSet::new();
        for tab in &self.tabs {
            if !tab_ids.insert(tab.id) {
                return Err(DomainError::DuplicateTab(tab.id));
            }
            tab.validate()?;
            for pane_id in tab.panes.keys() {
                if !pane_ids.insert(*pane_id) {
                    return Err(DomainError::DuplicatePane(*pane_id));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionState {
    pub schema_version: u32,
    pub active_workspace_id: WorkspaceId,
    pub workspaces: Vec<Workspace>,
    pub profiles: Vec<LaunchProfile>,
}

impl SessionState {
    #[must_use]
    pub fn new(workspace: Workspace, profiles: Vec<LaunchProfile>) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            active_workspace_id: workspace.id,
            workspaces: vec![workspace],
            profiles,
        }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedSchemaVersion(self.schema_version));
        }
        if !self
            .workspaces
            .iter()
            .any(|workspace| workspace.id == self.active_workspace_id)
        {
            return Err(DomainError::WorkspaceNotFound(self.active_workspace_id));
        }
        self.workspaces.iter().try_for_each(Workspace::validate)
    }

    pub fn add_workspace(&mut self, workspace: Workspace) -> Result<(), DomainError> {
        if self
            .workspaces
            .iter()
            .any(|candidate| candidate.id == workspace.id)
        {
            return Err(DomainError::DuplicateWorkspace(workspace.id));
        }
        self.active_workspace_id = workspace.id;
        self.workspaces.push(workspace);
        Ok(())
    }

    pub fn switch_workspace(&mut self, workspace_id: WorkspaceId) -> Result<(), DomainError> {
        if !self
            .workspaces
            .iter()
            .any(|workspace| workspace.id == workspace_id)
        {
            return Err(DomainError::WorkspaceNotFound(workspace_id));
        }
        self.active_workspace_id = workspace_id;
        Ok(())
    }

    pub fn rename_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        name: impl AsRef<str>,
    ) -> Result<(), DomainError> {
        let name = name.as_ref().trim();
        if name.is_empty() {
            return Err(DomainError::InvalidWorkspaceName);
        }
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or(DomainError::WorkspaceNotFound(workspace_id))?;
        workspace.name = name.to_owned();
        Ok(())
    }

    pub fn close_workspace(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceId, DomainError> {
        if self.workspaces.len() == 1 {
            return Err(DomainError::CannotCloseLastWorkspace);
        }
        let index = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
            .ok_or(DomainError::WorkspaceNotFound(workspace_id))?;
        self.workspaces.remove(index);
        if self.active_workspace_id == workspace_id {
            let next_index = index.min(self.workspaces.len() - 1);
            self.active_workspace_id = self.workspaces[next_index].id;
        }
        Ok(self.active_workspace_id)
    }

    pub fn move_tab(
        &mut self,
        tab_id: TabId,
        target_workspace_id: WorkspaceId,
        target_index: usize,
    ) -> Result<(), DomainError> {
        let source_workspace_index = self
            .workspaces
            .iter()
            .position(|workspace| workspace.tabs.iter().any(|tab| tab.id == tab_id))
            .ok_or(DomainError::TabNotFound(tab_id))?;
        let target_workspace_index = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == target_workspace_id)
            .ok_or(DomainError::WorkspaceNotFound(target_workspace_id))?;
        let source_tab_index = self.workspaces[source_workspace_index]
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .ok_or(DomainError::TabNotFound(tab_id))?;

        if source_workspace_index == target_workspace_index {
            let workspace = &mut self.workspaces[source_workspace_index];
            let tab = workspace.tabs.remove(source_tab_index);
            let insertion = target_index.min(workspace.tabs.len());
            workspace.tabs.insert(insertion, tab);
            return Ok(());
        }
        if self.workspaces[source_workspace_index].tabs.len() == 1 {
            return Err(DomainError::CannotMoveLastTab);
        }

        let tab = self.workspaces[source_workspace_index]
            .tabs
            .remove(source_tab_index);
        if self.workspaces[source_workspace_index].active_tab_id == tab_id {
            let next_index = source_tab_index.min(
                self.workspaces[source_workspace_index]
                    .tabs
                    .len()
                    .saturating_sub(1),
            );
            self.workspaces[source_workspace_index].active_tab_id =
                self.workspaces[source_workspace_index].tabs[next_index].id;
        }
        let target = &mut self.workspaces[target_workspace_index];
        let insertion = target_index.min(target.tabs.len());
        target.tabs.insert(insertion, tab);
        target.active_tab_id = tab_id;
        self.active_workspace_id = target_workspace_id;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct SessionStateWire {
    schema_version: u32,
    active_workspace_id: WorkspaceId,
    workspaces: Vec<WorkspaceWire>,
    profiles: Vec<LaunchProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WorkspaceWire {
    Current(Workspace),
    Legacy(LegacyWorkspace),
}

#[derive(Debug, Deserialize)]
struct LegacyWorkspace {
    id: WorkspaceId,
    name: String,
    root: PaneTree,
    panes: BTreeMap<PaneId, Pane>,
    focused_pane_id: PaneId,
}

impl<'de> Deserialize<'de> for SessionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SessionStateWire::deserialize(deserializer)?;
        if wire.schema_version != 1
            && wire.schema_version != 2
            && wire.schema_version != CURRENT_SCHEMA_VERSION
        {
            return Err(serde::de::Error::custom(format!(
                "session schema version {} is not supported",
                wire.schema_version
            )));
        }
        let workspaces = wire
            .workspaces
            .into_iter()
            .map(|workspace| match workspace {
                WorkspaceWire::Current(workspace) => workspace,
                WorkspaceWire::Legacy(legacy) => {
                    let tab = WorkspaceTab {
                        id: TabId::new(),
                        name: "Tab 1".into(),
                        root: legacy.root,
                        panes: legacy.panes,
                        focused_pane_id: legacy.focused_pane_id,
                    };
                    Workspace {
                        id: legacy.id,
                        name: legacy.name,
                        active_tab_id: tab.id,
                        tabs: vec![tab],
                    }
                }
            })
            .collect();
        Ok(Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            active_workspace_id: wire.active_workspace_id,
            workspaces,
            profiles: wire.profiles,
        })
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("split ratio {0} must be between 100 and 900 permille")]
    InvalidSplitRatio(u16),
    #[error("pane {0:?} was not found")]
    PaneNotFound(PaneId),
    #[error("tab {0:?} was not found")]
    TabNotFound(TabId),
    #[error("workspace {0:?} was not found")]
    WorkspaceNotFound(WorkspaceId),
    #[error("workspace {0:?} already exists")]
    DuplicateWorkspace(WorkspaceId),
    #[error("tab {0:?} already exists")]
    DuplicateTab(TabId),
    #[error("pane {0:?} appears in more than one tab")]
    DuplicatePane(PaneId),
    #[error("workspace names cannot be empty")]
    InvalidWorkspaceName,
    #[error("tab names cannot be empty")]
    InvalidTabName,
    #[error("the last workspace cannot be closed")]
    CannotCloseLastWorkspace,
    #[error("the last tab in a workspace cannot be closed")]
    CannotCloseLastTab,
    #[error("the last tab cannot be moved out of a workspace")]
    CannotMoveLastTab,
    #[error("workspace {0:?} must contain at least one tab")]
    WorkspaceHasNoTabs(WorkspaceId),
    #[error("the last pane in a tab cannot be closed")]
    CannotCloseLastPane,
    #[error("pane tree and pane map do not contain the same panes")]
    InconsistentPaneTree,
    #[error("pane {0:?} has an invalid active surface")]
    InvalidActiveSurface(PaneId),
    #[error("session schema version {0} is not supported")]
    UnsupportedSchemaVersion(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(profile_id: ProfileId, title: &str) -> Surface {
        Surface::terminal(
            title,
            TerminalSurface {
                profile_id,
                working_directory: None,
            },
        )
    }

    #[test]
    fn split_and_close_preserve_a_valid_tree() -> Result<(), Box<dyn std::error::Error>> {
        let profile_id = ProfileId::new();
        let mut workspace = Workspace::new("muxtrix", terminal(profile_id, "one"));
        let original = workspace
            .active_tab()
            .expect("new workspace should have an active tab")
            .focused_pane_id;
        let second = workspace.split_focused(
            SplitAxis::Horizontal,
            SplitRatio::EQUAL,
            terminal(profile_id, "two"),
        )?;
        let third = workspace.split_focused(
            SplitAxis::Vertical,
            SplitRatio::new(600)?,
            terminal(profile_id, "three"),
        )?;

        assert_eq!(
            workspace
                .active_tab()
                .expect("active tab should remain available")
                .root
                .pane_ids(),
            vec![original, second, third]
        );
        workspace.validate()?;
        workspace.close_pane(second)?;
        assert_eq!(
            workspace
                .active_tab()
                .expect("active tab should remain available")
                .root
                .pane_ids(),
            vec![original, third]
        );
        workspace.validate()?;
        Ok(())
    }

    #[test]
    fn last_pane_cannot_be_closed() {
        let profile_id = ProfileId::new();
        let mut workspace = Workspace::new("muxtrix", terminal(profile_id, "one"));
        let pane_id = workspace
            .active_tab()
            .expect("new workspace should have an active tab")
            .focused_pane_id;
        let result = workspace.close_pane(pane_id);
        assert_eq!(result, Err(DomainError::CannotCloseLastPane));
    }

    #[test]
    fn stacked_panes_preserve_order_when_split_and_closed() -> Result<(), DomainError> {
        let profile_id = ProfileId::new();
        let mut tab = WorkspaceTab::new("stack", terminal(profile_id, "one"));
        let first = tab.focused_pane_id;
        let second = tab.split_focused(
            SplitAxis::Horizontal,
            SplitRatio::EQUAL,
            terminal(profile_id, "two"),
        )?;
        let third = tab.split_focused(
            SplitAxis::Vertical,
            SplitRatio::EQUAL,
            terminal(profile_id, "three"),
        )?;
        tab.root = PaneTree::stack(vec![first, second, third]).expect("three panes form a stack");

        tab.close_pane(second)?;
        assert_eq!(tab.root.pane_ids(), vec![first, third]);
        assert!(matches!(tab.root, PaneTree::Stack { .. }));

        tab.focused_pane_id = third;
        let fourth = tab.split_focused(
            SplitAxis::Horizontal,
            SplitRatio::EQUAL,
            terminal(profile_id, "four"),
        )?;
        assert_eq!(tab.root.pane_ids(), vec![first, third, fourth]);
        assert!(matches!(tab.root, PaneTree::Split { .. }));
        tab.validate()
    }

    #[test]
    fn tabs_can_be_added_switched_reordered_and_closed() -> Result<(), DomainError> {
        let profile_id = ProfileId::new();
        let first = Workspace::new("first", terminal(profile_id, "one"));
        let first_workspace_id = first.id;
        let first_tab_id = first.active_tab_id;
        let second = Workspace::new("second", terminal(profile_id, "two"));
        let second_workspace_id = second.id;
        let mut session = SessionState::new(first, Vec::new());
        session.add_workspace(second)?;

        let extra = WorkspaceTab::new("Tab 2", terminal(profile_id, "three"));
        let extra_id = extra.id;
        session.workspaces[0].add_tab(extra)?;
        session.move_tab(extra_id, second_workspace_id, 0)?;

        assert_eq!(session.active_workspace_id, second_workspace_id);
        assert_eq!(session.workspaces[0].tabs.len(), 1);
        assert_eq!(session.workspaces[0].active_tab_id, first_tab_id);
        assert_eq!(session.workspaces[1].tabs[0].id, extra_id);
        assert_eq!(session.workspaces[1].active_tab_id, extra_id);
        assert_eq!(
            session.move_tab(first_tab_id, second_workspace_id, 0),
            Err(DomainError::CannotMoveLastTab)
        );

        let removed = session.workspaces[1].close_tab(extra_id)?;
        assert_eq!(removed.id, extra_id);
        assert_eq!(session.workspaces[1].tabs.len(), 1);
        assert_eq!(session.workspaces[0].id, first_workspace_id);
        session.validate()
    }

    #[test]
    fn tabs_and_panes_can_carry_user_chosen_names() -> Result<(), Box<dyn std::error::Error>> {
        let profile_id = ProfileId::new();
        let mut workspace = Workspace::new("muxtrix", terminal(profile_id, "shell"));
        let tab_id = workspace.active_tab_id;
        workspace.rename_tab(tab_id, "  review  ")?;
        assert_eq!(workspace.tabs[0].name, "review");
        assert_eq!(
            workspace.rename_tab(tab_id, "   "),
            Err(DomainError::InvalidTabName)
        );

        let pane_id = workspace.tabs[0].focused_pane_id;
        workspace
            .pane_mut(pane_id)
            .expect("pane should exist")
            .custom_name = Some("build watcher".into());
        workspace
            .pane_mut(pane_id)
            .expect("pane should exist")
            .agent = Some(PaneAgent::OhMyPi);
        let encoded = serde_json::to_string(&workspace)?;
        let decoded: Workspace = serde_json::from_str(&encoded)?;
        let pane = decoded
            .pane(pane_id)
            .expect("pane should survive the round trip");
        assert_eq!(pane.custom_name.as_deref(), Some("build watcher"));
        assert_eq!(pane.agent, Some(PaneAgent::OhMyPi));
        Ok(())
    }

    #[test]
    fn version_one_sessions_migrate_to_a_default_tab() -> Result<(), Box<dyn std::error::Error>> {
        let profile_id = ProfileId::new();
        let workspace = WorkspaceTab::new("legacy", terminal(profile_id, "shell"));
        let workspace_id = WorkspaceId::new();
        let legacy = serde_json::json!({
            "schema_version": 1,
            "active_workspace_id": workspace_id,
            "workspaces": [{
                "id": workspace_id,
                "name": "legacy",
                "root": workspace.root,
                "panes": workspace.panes,
                "focused_pane_id": workspace.focused_pane_id,
            }],
            "profiles": [],
        });

        let migrated: SessionState = serde_json::from_value(legacy)?;
        assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(migrated.workspaces[0].tabs.len(), 1);
        assert_eq!(migrated.workspaces[0].tabs[0].name, "Tab 1");
        migrated.validate()?;
        Ok(())
    }

    #[test]
    fn version_two_sessions_migrate_without_changing_their_panes()
    -> Result<(), Box<dyn std::error::Error>> {
        let profile_id = ProfileId::new();
        let workspace = Workspace::new("existing", terminal(profile_id, "shell"));
        let pane_id = workspace.tabs[0].focused_pane_id;
        let legacy = serde_json::json!({
            "schema_version": 2,
            "active_workspace_id": workspace.id,
            "workspaces": [workspace],
            "profiles": [],
        });

        let migrated: SessionState = serde_json::from_value(legacy)?;
        assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            migrated.workspaces[0].tabs[0].root.pane_ids(),
            vec![pane_id]
        );
        migrated.validate()?;
        Ok(())
    }

    #[test]
    fn split_ratio_rejects_invalid_serialized_state() {
        let result = serde_json::from_str::<SplitRatio>("99");
        assert!(result.is_err());
    }

    #[test]
    fn session_round_trips_through_json() -> Result<(), Box<dyn std::error::Error>> {
        let profile = LaunchProfile {
            id: ProfileId::new(),
            name: "WSL Ubuntu".into(),
            backend: ProcessBackend::Wsl {
                distribution: Some("Ubuntu-22.04".into()),
            },
            program: "bash".into(),
            arguments: vec!["-l".into()],
            working_directory: Some(PathBuf::from("/home/user/project")),
        };
        let workspace = Workspace::new("project", terminal(profile.id, "shell"));
        let state = SessionState::new(workspace, vec![profile]);

        let encoded = serde_json::to_string_pretty(&state)?;
        let decoded: SessionState = serde_json::from_str(&encoded)?;
        assert_eq!(decoded, state);
        decoded.validate()?;
        Ok(())
    }

    #[test]
    fn workspaces_can_be_added_switched_renamed_and_closed() -> Result<(), DomainError> {
        let profile_id = ProfileId::new();
        let first = Workspace::new("first", terminal(profile_id, "one"));
        let first_id = first.id;
        let second = Workspace::new("second", terminal(profile_id, "two"));
        let second_id = second.id;
        let mut session = SessionState::new(first, Vec::new());

        session.add_workspace(second)?;
        assert_eq!(session.active_workspace_id, second_id);
        session.rename_workspace(second_id, "  project beta  ")?;
        assert_eq!(session.workspaces[1].name, "project beta");
        session.switch_workspace(first_id)?;
        assert_eq!(session.active_workspace_id, first_id);
        session.close_workspace(first_id)?;
        assert_eq!(session.active_workspace_id, second_id);
        assert_eq!(session.workspaces.len(), 1);
        session.validate()
    }

    #[test]
    fn last_workspace_and_blank_names_are_rejected() {
        let profile_id = ProfileId::new();
        let workspace = Workspace::new("first", terminal(profile_id, "one"));
        let workspace_id = workspace.id;
        let mut session = SessionState::new(workspace, Vec::new());

        assert_eq!(
            session.rename_workspace(workspace_id, "   "),
            Err(DomainError::InvalidWorkspaceName)
        );
        assert_eq!(
            session.close_workspace(workspace_id),
            Err(DomainError::CannotCloseLastWorkspace)
        );
    }
}
