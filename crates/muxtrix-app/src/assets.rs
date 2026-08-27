//! Icons, served to GPUI from the binary.
//!
//! SVG files are reachable by path because GPUI resolves `svg().path(..)`
//! through an [`AssetSource`] rather than taking bytes directly. Nothing is
//! read from disk: an installed Muxtrix has no icon directory beside it.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

use crate::app::IconKind;

/// Every icon, paired with the path a view asks for it by.
///
/// One table rather than a `match` per lookup so `list` and `load` cannot
/// disagree about what exists.
const ICONS: &[(&str, &[u8])] = &[
    ("icons/back.svg", include_bytes!("../assets/icons/back.svg")),
    (
        "icons/forward.svg",
        include_bytes!("../assets/icons/forward.svg"),
    ),
    ("icons/add.svg", include_bytes!("../assets/icons/add.svg")),
    (
        "icons/collapse.svg",
        include_bytes!("../assets/icons/collapse.svg"),
    ),
    (
        "icons/expand.svg",
        include_bytes!("../assets/icons/expand.svg"),
    ),
    (
        "icons/chevron-right.svg",
        include_bytes!("../assets/icons/chevron-right.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/split-right.svg",
        include_bytes!("../assets/icons/split-right.svg"),
    ),
    (
        "icons/split-down.svg",
        include_bytes!("../assets/icons/split-down.svg"),
    ),
    (
        "icons/maximize.svg",
        include_bytes!("../assets/icons/maximize.svg"),
    ),
    (
        "icons/restore.svg",
        include_bytes!("../assets/icons/restore.svg"),
    ),
    (
        "icons/settings.svg",
        include_bytes!("../assets/icons/settings.svg"),
    ),
    (
        "icons/command.svg",
        include_bytes!("../assets/icons/command.svg"),
    ),
    (
        "icons/github.svg",
        include_bytes!("../assets/icons/github.svg"),
    ),
    (
        "icons/refresh.svg",
        include_bytes!("../assets/icons/refresh.svg"),
    ),
    (
        "icons/branch.svg",
        include_bytes!("../assets/icons/branch.svg"),
    ),
    ("icons/file.svg", include_bytes!("../assets/icons/file.svg")),
    (
        "icons/close.svg",
        include_bytes!("../assets/icons/close.svg"),
    ),
    (
        "icons/overflow.svg",
        include_bytes!("../assets/icons/overflow.svg"),
    ),
    (
        "icons/status-ready.svg",
        include_bytes!("../assets/icons/status-ready.svg"),
    ),
    (
        "icons/status-warning.svg",
        include_bytes!("../assets/icons/status-warning.svg"),
    ),
    (
        "icons/status-error.svg",
        include_bytes!("../assets/icons/status-error.svg"),
    ),
    (
        "icons/status-info.svg",
        include_bytes!("../assets/icons/status-info.svg"),
    ),
    (
        "icons/pull-request-open.svg",
        include_bytes!("../assets/icons/pull-request-open.svg"),
    ),
    (
        "icons/pull-request-draft.svg",
        include_bytes!("../assets/icons/pull-request-draft.svg"),
    ),
    (
        "icons/pull-request-closed.svg",
        include_bytes!("../assets/icons/pull-request-closed.svg"),
    ),
    (
        "icons/pull-request-merged.svg",
        include_bytes!("../assets/icons/pull-request-merged.svg"),
    ),
    (
        "icons/package.svg",
        include_bytes!("../assets/icons/package.svg"),
    ),
    (
        "icons/package-open.svg",
        include_bytes!("../assets/icons/package-open.svg"),
    ),
    (
        "icons/app-window.svg",
        include_bytes!("../assets/icons/app-window.svg"),
    ),
    (
        "icons/folder-git.svg",
        include_bytes!("../assets/icons/folder-git.svg"),
    ),
];

/// The asset path for an icon, as `svg().path(..)` wants it.
pub(crate) fn icon_path(kind: IconKind) -> SharedString {
    let path = match kind {
        IconKind::Back => "icons/back.svg",
        IconKind::Forward => "icons/forward.svg",
        IconKind::Add => "icons/add.svg",
        IconKind::Collapse => "icons/collapse.svg",
        IconKind::Expand => "icons/expand.svg",
        IconKind::SplitRight => "icons/split-right.svg",
        IconKind::SplitDown => "icons/split-down.svg",
        IconKind::Maximize => "icons/maximize.svg",
        IconKind::Restore => "icons/restore.svg",
        IconKind::Settings => "icons/settings.svg",
        IconKind::Command => "icons/command.svg",
        IconKind::GitHub => "icons/github.svg",
        IconKind::Refresh => "icons/refresh.svg",
        IconKind::Branch => "icons/branch.svg",
        IconKind::File => "icons/file.svg",
        IconKind::Close => "icons/close.svg",
        IconKind::Overflow => "icons/overflow.svg",
        IconKind::StatusReady => "icons/status-ready.svg",
        IconKind::StatusWarning => "icons/status-warning.svg",
        IconKind::StatusError => "icons/status-error.svg",
        IconKind::StatusInfo => "icons/status-info.svg",
        IconKind::PullRequestOpen => "icons/pull-request-open.svg",
        IconKind::PullRequestDraft => "icons/pull-request-draft.svg",
        IconKind::PullRequestClosed => "icons/pull-request-closed.svg",
        IconKind::PullRequestMerged => "icons/pull-request-merged.svg",
        IconKind::Package => "icons/package.svg",
        IconKind::PackageOpen => "icons/package-open.svg",
        IconKind::AppWindow => "icons/app-window.svg",
        IconKind::FolderGit => "icons/folder-git.svg",
    };
    SharedString::new_static(path)
}

/// Serves the embedded icons to GPUI.
pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::new_static(name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon a view can name has to be servable, or the icon silently
    /// renders as nothing.
    #[test]
    fn every_icon_kind_resolves_to_a_served_asset() {
        let kinds = [
            IconKind::Back,
            IconKind::Forward,
            IconKind::Add,
            IconKind::Collapse,
            IconKind::Expand,
            IconKind::SplitRight,
            IconKind::SplitDown,
            IconKind::Maximize,
            IconKind::Restore,
            IconKind::Settings,
            IconKind::Command,
            IconKind::GitHub,
            IconKind::Refresh,
            IconKind::Branch,
            IconKind::File,
            IconKind::Close,
            IconKind::Overflow,
            IconKind::StatusReady,
            IconKind::StatusWarning,
            IconKind::StatusError,
            IconKind::StatusInfo,
            IconKind::PullRequestOpen,
            IconKind::PullRequestDraft,
            IconKind::PullRequestClosed,
            IconKind::PullRequestMerged,
            IconKind::Package,
            IconKind::PackageOpen,
            IconKind::AppWindow,
            IconKind::FolderGit,
        ];
        for kind in kinds {
            let path = icon_path(kind);
            assert!(
                ICONS.iter().any(|(name, _)| *name == path.as_ref()),
                "{kind:?} names {path}, which no asset serves"
            );
        }
    }
}
