//! The worktree manager page.
//!
//! A table of every discovered worktree with its branch, age and unpushed
//! commits, plus the bulk actions that reclaim unused ones.

use iced::widget::column;

use crate::views::prelude::*;

use crate::app::{
    SETTINGS_PAGE_PADDING_X, SettingsButtonKind, WORKTREE_LANE_SPACING, WORKTREE_PAGE_MAX_WIDTH,
    WORKTREE_ROW_PADDING_X, WorktreeLanes, WorktreeManagerEntry, app_tooltip,
    centered_button_content, centered_button_label, ellipsize, ellipsize_start, ruled_surface,
    selection_bar, settings_action_button, settings_button_style, settings_divider,
    settings_notice, single_line_ellipsize, unused_worktree_paths, worktree_display_name,
    worktree_footer_hint, worktree_mono_budget, worktree_status_tag, worktree_table_header,
    worktree_ui_budget,
};
use iced::Font;

impl Muxtrix {
    pub(crate) fn worktree_settings_view(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings_draft.appearance);
        let compact = self.window_size.width < 900.0;
        let lanes = WorktreeLanes::for_window(self.window_size.width, compact);
        let Some(manager) = self.worktree_manager.as_ref() else {
            return container(settings_notice(
                "Worktrees are not loaded",
                "Choose Refresh to inspect the focused terminal's repository.",
                "Muxtrix only reads local Git metadata and never fetches from a remote.",
                tokens.muted,
                &self.settings_draft,
            ))
            .padding(28)
            .width(Fill)
            .height(Fill)
            .into();
        };
        let unused_count = unused_worktree_paths(&manager.entries).len();
        let repository = manager
            .repo_root
            .as_ref()
            .map(|root| format!("{} · {}", worktree_display_name(root), root.display()));
        let heading = row![
            column![
                text("Worktrees")
                    .size(self.settings_draft.ui_pixels(22.0))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::DEFAULT
                    }),
                text(repository.unwrap_or_else(|| {
                    "Registered checkouts for the focused terminal's repository".to_owned()
                }))
                .size(self.settings_draft.ui_pixels(10.0))
                .color(tokens.muted),
            ]
            .spacing(4)
            .width(Fill),
            settings_action_button(
                "Refresh",
                Message::RefreshWorktreeManager,
                SettingsButtonKind::Secondary,
                &self.settings_draft,
            ),
            {
                let label = if manager.busy {
                    "Removing…".to_owned()
                } else {
                    format!("Remove unused ({unused_count})")
                };
                let mut button = button(centered_button_content(
                    text(label).size(self.settings_draft.ui_pixels(9.0)),
                ))
                .height(30)
                .padding([0, 11])
                .style(move |_, status| {
                    settings_button_style(tokens, SettingsButtonKind::Danger, status)
                });
                if unused_count > 0 && !manager.busy && !manager.loading {
                    button = button.on_press(Message::WorktreeManagerDeleteUnused);
                }
                button
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let mut page = column![heading].spacing(18);
        if manager.loading {
            page = page.push(settings_notice(
                "Loading repository",
                "Discovering registered worktrees and checking local-only commits in the background.",
                "You can return to the terminal immediately; this screen will update when discovery finishes.",
                tokens.accent,
                &self.settings_draft,
            ));
        } else if let Some(failure) = &manager.failure {
            page = page.push(settings_notice(
                "Worktrees unavailable",
                failure,
                "Focus a terminal inside a Git repository, then choose Refresh.",
                tokens.warning,
                &self.settings_draft,
            ));
        } else {
            if let Some(error) = &manager.error {
                page = page.push(settings_notice(
                    "Worktree action failed",
                    error,
                    "Nothing else was changed. Resolve the Git issue and choose Refresh to try again.",
                    tokens.danger,
                    &self.settings_draft,
                ));
            }
            if manager.entries.is_empty() {
                page = page.push(settings_notice(
                    "No registered worktrees",
                    "This repository only has its current checkout, or Git returned an empty worktree list.",
                    "Create a checkout from the command palette with New worktree pane or New worktree tab.",
                    tokens.muted,
                    &self.settings_draft,
                ));
            } else {
                let mut rows = column![];
                if !compact {
                    rows = rows.push(worktree_table_header(&self.settings_draft, lanes));
                }
                for (index, entry) in manager.entries.iter().enumerate() {
                    if index > 0 || !compact {
                        rows = rows.push(settings_divider(tokens));
                    }
                    rows = rows.push(self.worktree_settings_row(index, entry, compact, lanes));
                }
                page = page.push(
                    column![
                        row![
                            text(format!(
                                "{} registered {}",
                                manager.entries.len(),
                                if manager.entries.len() == 1 {
                                    "checkout"
                                } else {
                                    "checkouts"
                                }
                            ))
                            .size(self.settings_draft.ui_pixels(11.0))
                            .font(Font {
                                weight: font::Weight::Semibold,
                                ..Font::DEFAULT
                            }),
                            container("").width(Fill),
                            text("Local status only · no network fetch")
                                .size(self.settings_draft.ui_pixels(8.5))
                                .color(tokens.faint),
                        ]
                        .align_y(Alignment::Center),
                        container(rows).width(Fill).style(move |_| {
                            container::Style::default()
                                .background(tokens.panel)
                                .border(Border {
                                    color: tokens.line,
                                    width: 1.0,
                                    radius: 6.0.into(),
                                })
                        }),
                    ]
                    .spacing(8),
                );
            }
        }
        let hint = |keys: &'static str, label: &'static str| {
            worktree_footer_hint(keys, label, &self.settings_draft)
        };
        // A key is only advertised while it can act. With nothing listed —
        // still loading, unavailable, or genuinely empty — the selection and
        // removal chords would be claims the page cannot honour.
        let navigable =
            !manager.loading && manager.failure.is_none() && !manager.entries.is_empty();
        let mut hints = row![]
            .spacing(if compact { 16 } else { 20 })
            .align_y(Alignment::Center);
        if navigable {
            hints = hints.push(hint("↑↓", "Select")).push(hint(
                "Del",
                if compact { "Remove" } else { "Remove checkout" },
            ));
        }
        hints = hints.push(hint(
            "Esc",
            if compact {
                "Terminal"
            } else {
                "Back to terminal"
            },
        ));
        let footer_content: Element<'_, Message> = if compact || !navigable {
            hints.into()
        } else {
            row![
                hints,
                container("").width(Fill),
                text("Protected and in-use worktrees cannot be removed")
                    .size(self.settings_draft.ui_pixels(9.0))
                    .color(tokens.faint)
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(20)
            .align_y(Alignment::Center)
            .into()
        };
        let footer = container(footer_content)
            .width(Fill)
            .height(44)
            .align_y(iced::alignment::Vertical::Center)
            .padding([0.0, SETTINGS_PAGE_PADDING_X])
            .style(move |_| ruled_surface(tokens.rail, tokens.line));
        column![
            scrollable(
                container(page.max_width(WORKTREE_PAGE_MAX_WIDTH))
                    .padding([24.0, SETTINGS_PAGE_PADDING_X])
                    .center_x(Fill)
            )
            .height(Fill),
            footer,
        ]
        .height(Fill)
        .into()
    }

    pub(crate) fn worktree_settings_row(
        &self,
        index: usize,
        entry: &WorktreeManagerEntry,
        compact: bool,
        lanes: WorktreeLanes,
    ) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings_draft.appearance);
        let selected = self
            .worktree_manager
            .as_ref()
            .is_some_and(|manager| manager.selected == index);
        let name = worktree_display_name(&entry.path);
        let branch = entry
            .branch
            .as_deref()
            .unwrap_or("Detached HEAD")
            .to_owned();
        let status = if let Some(blocker) = &entry.deletion_blocker {
            worktree_status_tag(blocker, tokens.faint, &self.settings_draft)
        } else if entry.used_by.is_some() {
            worktree_status_tag("In use", tokens.warning, &self.settings_draft)
        } else {
            worktree_status_tag("Available", tokens.muted, &self.settings_draft)
        };
        let status_detail = entry
            .used_by
            .as_deref()
            .unwrap_or(if entry.deletion_blocker.is_some() {
                "Removal disabled"
            } else {
                "Not used by an open pane"
            })
            .to_owned();
        let commit_color = if entry.unpushed_commits > 0 {
            tokens.warning
        } else {
            tokens.faint
        };
        let commit_copy = if entry.unpushed_commits == 0 {
            "None".to_owned()
        } else {
            format!(
                "{} local-only {}",
                entry.unpushed_commits,
                if entry.unpushed_commits == 1 {
                    "commit"
                } else {
                    "commits"
                }
            )
        };
        // A lane headed ACTION names one action. The reason a row cannot act
        // is state, and state already has its own lane immediately to the
        // left — relabelling the button with it printed "In use" twice in one
        // row and made the lane's shape change from row to row. The button
        // keeps its label and goes disabled instead, with the reason repeated
        // in a tooltip so hovering the dead control still explains itself.
        let blocked_reason = entry
            .deletion_blocker
            .as_deref()
            .map(|blocker| format!("Protected: {blocker} cannot be removed"))
            .or_else(|| {
                entry
                    .used_by
                    .as_deref()
                    .map(|pane| format!("In use by {pane}; close that pane first"))
            });
        let removing = self
            .worktree_manager
            .as_ref()
            .is_some_and(|manager| manager.busy && selected);
        let delete_label = if removing { "Removing…" } else { "Remove" };
        let mut delete = button(centered_button_label(
            delete_label,
            self.settings_draft.ui_pixels(9.0),
        ))
        .height(28)
        .padding([0, 12])
        .style(move |_, button_status| {
            settings_button_style(tokens, SettingsButtonKind::Danger, button_status)
        });
        if blocked_reason.is_none()
            && self
                .worktree_manager
                .as_ref()
                .is_some_and(|manager| !manager.busy)
        {
            delete = delete.on_press(Message::WorktreeManagerDelete(index));
        }
        let delete: Element<'_, Message> = match blocked_reason {
            Some(reason) => app_tooltip(
                delete,
                reason,
                tooltip::Position::Left,
                tokens,
                self.settings_draft.ui_pixels(9.0),
            ),
            None => delete.into(),
        };
        // Every string on the row is budgeted against the lane that holds it,
        // so copy ends in an ellipsis inside its own lane instead of sliding
        // under the next one and being cut mid-glyph.
        let location = entry.path.parent().map_or_else(
            || entry.path.display().to_string(),
            |parent| parent.display().to_string(),
        );
        let name_size = self.settings_draft.ui_pixels(11.0);
        let path_size = self.settings_draft.ui_pixels(8.0);
        let branch_size = self.settings_draft.ui_pixels(9.0);
        let detail_size = self.settings_draft.ui_pixels(8.0);
        let identity = column![
            text(ellipsize(
                &name,
                worktree_ui_budget(lanes.identity, name_size)
            ))
            .size(name_size)
            .font(Font {
                weight: font::Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(tokens.text)
            .wrapping(iced::widget::text::Wrapping::None),
            // The name above is already this path's last segment, so the
            // secondary line carries the directory that holds it — together
            // they still spell the full path, without printing the leaf
            // twice. Checkout locations share a long common prefix, so the
            // front is what can be spent.
            text(ellipsize_start(
                &location,
                worktree_mono_budget(lanes.identity, path_size, &self.settings_draft),
            ))
            .font(self.settings_draft.terminal_font.iced())
            .size(path_size)
            .color(tokens.faint)
            .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(3);
        let status_column = column![
            status,
            text(single_line_ellipsize(
                &status_detail,
                worktree_ui_budget(lanes.status, detail_size)
            ))
            .size(detail_size)
            .color(tokens.faint)
            .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(4);
        let commit_column = column![
            text(commit_copy)
                .size(branch_size)
                .color(commit_color)
                .wrapping(iced::widget::text::Wrapping::None),
            text(if entry.unpushed_commits > 0 {
                "Not on any remote ref"
            } else {
                "Safe from local-only loss"
            })
            .size(detail_size)
            .color(tokens.faint)
            .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(3);
        let branch_text = |width: f32| {
            text(ellipsize(
                &branch,
                worktree_mono_budget(width, branch_size, &self.settings_draft),
            ))
            .font(self.settings_draft.terminal_font.iced())
            .size(branch_size)
            .wrapping(iced::widget::text::Wrapping::None)
        };
        let row_content: Element<'_, Message> = if compact {
            column![
                identity,
                row![
                    // The label leads its value here: with the lanes stacked,
                    // a bare slug has no column header to name it.
                    column![
                        text("Branch")
                            .size(detail_size)
                            .color(tokens.faint)
                            .wrapping(iced::widget::text::Wrapping::None),
                        branch_text(lanes.branch),
                    ]
                    .spacing(3)
                    .width(Length::FillPortion(1)),
                    status_column.width(Length::FillPortion(1)),
                ]
                .spacing(WorktreeLanes::STACKED_GAP),
                row![
                    column![
                        text("Local commits")
                            .size(detail_size)
                            .color(tokens.faint)
                            .wrapping(iced::widget::text::Wrapping::None),
                        commit_column,
                    ]
                    .spacing(3)
                    .width(Fill),
                    delete,
                ]
                .spacing(WorktreeLanes::STACKED_GAP)
                .align_y(Alignment::Center),
            ]
            .spacing(10)
            .into()
        } else {
            row![
                container(identity).width(lanes.identity).clip(true),
                container(branch_text(lanes.branch))
                    .width(lanes.branch)
                    .clip(true),
                container(status_column).width(lanes.status).clip(true),
                container(commit_column).width(lanes.commits).clip(true),
                container(delete)
                    .width(lanes.action)
                    .align_x(iced::alignment::Horizontal::Right),
            ]
            .spacing(WORKTREE_LANE_SPACING)
            .align_y(Alignment::Center)
            .into()
        };
        container(row![
            selection_bar(selected, tokens),
            container(row_content)
                .padding([12.0, WORKTREE_ROW_PADDING_X])
                .width(Fill)
        ])
        .width(Fill)
        .style(move |_| {
            container::Style::default().background(if selected {
                Color {
                    a: 0.10,
                    ..tokens.accent
                }
            } else {
                Color::TRANSPARENT
            })
        })
        .into()
    }
}
