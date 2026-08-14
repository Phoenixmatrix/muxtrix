use muxtrix_control::Agent;
use muxtrix_domain::SplitAxis;

/// Ghostty's default clipboard chords, shown wherever the actions surface.
pub(crate) const COPY_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "Cmd+C"
} else {
    "Ctrl+Shift+C"
};
pub(crate) const PASTE_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "Cmd+V"
} else {
    "Ctrl+Shift+V"
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandAction {
    Split(SplitAxis),
    GrowPane,
    RestorePaneSize,
    PreviousPaneLayout,
    NextPaneLayout,
    ClosePane,
    CopySelection,
    PasteClipboard,
    NewTab,
    CloseTab,
    NewWorkspace,
    CloseWorkspace,
    RenameWorkspace,
    RenameTab,
    RenamePane,
    NewWorktree(WorktreeKind),
    NewWorktreeWithAgent(WorktreeKind),
    RestartPaneInWorktree,
    RestartPaneInExistingWorktree,
    RestartPaneInWorktreeWithAgent,
    RestartPaneInExistingWorktreeWithAgent,
    ManageWorktrees,
    ManageSessions,
    FleetTabs,
    FleetAgents,
    FleetRepos,
    OpenGitHubPanel,
    OpenSettings,
    LaunchAgent(Agent),
    ReturnToWorkspace,
}

impl CommandAction {
    /// Actions that need the tiled pane tree to be visible and interactive.
    /// A maximized pane is a temporary modal projection, so mutating that
    /// hidden tree would make the command appear to do nothing.
    pub(crate) fn requires_tiled_panes(self) -> bool {
        matches!(
            self,
            Self::Split(_)
                | Self::GrowPane
                | Self::RestorePaneSize
                | Self::PreviousPaneLayout
                | Self::NextPaneLayout
                | Self::NewWorktree(WorktreeKind::Pane(_))
                | Self::NewWorktreeWithAgent(WorktreeKind::Pane(_))
                | Self::LaunchAgent(_)
        )
    }

    pub(crate) fn requires_default_agent(self) -> bool {
        matches!(
            self,
            Self::NewWorktreeWithAgent(_)
                | Self::RestartPaneInWorktreeWithAgent
                | Self::RestartPaneInExistingWorktreeWithAgent
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeKind {
    Pane(SplitAxis),
    Tab,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Command {
    pub(crate) title: &'static str,
    pub(crate) subtitle: &'static str,
    pub(crate) keywords: &'static str,
    pub(crate) shortcut: &'static str,
    pub(crate) action: CommandAction,
}

const COMMANDS: [Command; 36] = [
    Command {
        title: "Split pane right",
        subtitle: "Open an independent terminal beside the focused pane",
        keywords: "terminal horizontal new side by side",
        shortcut: "Ctrl+Shift+E",
        action: CommandAction::Split(SplitAxis::Horizontal),
    },
    Command {
        title: "Split pane down",
        subtitle: "Open an independent terminal below the focused pane",
        keywords: "terminal vertical new above below",
        shortcut: "Ctrl+Shift+O",
        action: CommandAction::Split(SplitAxis::Vertical),
    },
    Command {
        title: "Grow focused pane",
        subtitle: "Enlarge the pane, stacking neighbors when it runs out of room",
        keywords: "resize increase grow stack pane larger",
        shortcut: "Ctrl++",
        action: CommandAction::GrowPane,
    },
    Command {
        title: "Restore previous pane size",
        subtitle: "Step back through focused-pane resize changes",
        keywords: "resize decrease shrink undo pane smaller",
        shortcut: "Ctrl+-",
        action: CommandAction::RestorePaneSize,
    },
    Command {
        title: "Previous pane layout",
        subtitle: "Cycle backward through Base, Vertical, Horizontal, and stacked layouts",
        keywords: "layout previous swap tiled vertical horizontal stacked",
        shortcut: "Alt+[",
        action: CommandAction::PreviousPaneLayout,
    },
    Command {
        title: "Next pane layout",
        subtitle: "Cycle forward through Base, Vertical, Horizontal, and stacked layouts",
        keywords: "layout next swap tiled vertical horizontal stacked",
        shortcut: "Alt+]",
        action: CommandAction::NextPaneLayout,
    },
    Command {
        title: "Close focused pane",
        subtitle: "Close the active terminal surface",
        keywords: "remove terminal kill",
        shortcut: "",
        action: CommandAction::ClosePane,
    },
    Command {
        title: "Copy",
        subtitle: "Copy the focused terminal selection to the clipboard",
        keywords: "clipboard copy selection terminal",
        shortcut: COPY_SHORTCUT,
        action: CommandAction::CopySelection,
    },
    Command {
        title: "Paste",
        subtitle: "Paste the clipboard into the focused terminal",
        keywords: "clipboard paste insert terminal",
        shortcut: PASTE_SHORTCUT,
        action: CommandAction::PasteClipboard,
    },
    Command {
        title: "New tab",
        subtitle: "Create a new tab with one terminal pane",
        keywords: "tab terminal new create",
        shortcut: "Ctrl+Shift+T",
        action: CommandAction::NewTab,
    },
    Command {
        title: "Close tab",
        subtitle: "Close the active tab and all of its panes",
        keywords: "tab terminal remove close",
        shortcut: "",
        action: CommandAction::CloseTab,
    },
    Command {
        title: "New workspace",
        subtitle: "Create and switch to a separate terminal workspace",
        keywords: "workspace project new create",
        shortcut: "",
        action: CommandAction::NewWorkspace,
    },
    Command {
        title: "Close workspace",
        subtitle: "Close the current workspace and all of its terminals",
        keywords: "workspace project remove close",
        shortcut: "",
        action: CommandAction::CloseWorkspace,
    },
    Command {
        title: "Rename workspace",
        subtitle: "Give the current workspace a new name",
        keywords: "workspace edit title change name",
        shortcut: "",
        action: CommandAction::RenameWorkspace,
    },
    Command {
        title: "Rename tab",
        subtitle: "Give the active tab a new name",
        keywords: "tab edit title change name",
        shortcut: "",
        action: CommandAction::RenameTab,
    },
    Command {
        title: "Rename pane",
        subtitle: "Set a custom pane name; leave it empty to restore the automatic title",
        keywords: "pane edit title change name terminal",
        shortcut: "",
        action: CommandAction::RenamePane,
    },
    Command {
        title: "New worktree pane right",
        subtitle: "Create a git worktree and open it beside the focused pane",
        keywords: "git branch worktree checkout pane split horizontal right beside",
        shortcut: "",
        action: CommandAction::NewWorktree(WorktreeKind::Pane(SplitAxis::Horizontal)),
    },
    Command {
        title: "New worktree pane down",
        subtitle: "Create a git worktree and open it below the focused pane",
        keywords: "git branch worktree checkout pane split vertical down below",
        shortcut: "",
        action: CommandAction::NewWorktree(WorktreeKind::Pane(SplitAxis::Vertical)),
    },
    Command {
        title: "New worktree with agent pane right",
        subtitle: "Create a git worktree beside the focused pane and start the default agent",
        keywords: "git branch worktree agent checkout pane split horizontal right beside",
        shortcut: "",
        action: CommandAction::NewWorktreeWithAgent(WorktreeKind::Pane(SplitAxis::Horizontal)),
    },
    Command {
        title: "New worktree with agent pane down",
        subtitle: "Create a git worktree below the focused pane and start the default agent",
        keywords: "git branch worktree agent checkout pane split vertical down below",
        shortcut: "",
        action: CommandAction::NewWorktreeWithAgent(WorktreeKind::Pane(SplitAxis::Vertical)),
    },
    Command {
        title: "New worktree tab",
        subtitle: "Create a git worktree from the focused pane's repository and open it in a tab",
        keywords: "git branch worktree checkout tab",
        shortcut: "",
        action: CommandAction::NewWorktree(WorktreeKind::Tab),
    },
    Command {
        title: "Restart pane in worktree…",
        subtitle: "Create a Git worktree and restart the focused terminal there",
        keywords: "git branch worktree new create current pane restart replace terminal cwd directory",
        shortcut: "",
        action: CommandAction::RestartPaneInWorktree,
    },
    Command {
        title: "Restart pane in existing worktree…",
        subtitle: "Choose a registered Git worktree and restart the focused terminal there",
        keywords: "git worktree existing reuse select switch change current pane restart replace terminal cwd directory",
        shortcut: "",
        action: CommandAction::RestartPaneInExistingWorktree,
    },
    Command {
        title: "Restart pane in new worktree with agent…",
        subtitle: "Create a Git worktree, restart this pane there, and start the default agent",
        keywords: "git branch worktree agent new create current pane restart replace terminal cwd directory",
        shortcut: "",
        action: CommandAction::RestartPaneInWorktreeWithAgent,
    },
    Command {
        title: "Restart pane in existing worktree with agent…",
        subtitle: "Choose a registered Git worktree, restart this pane there, and start the default agent",
        keywords: "git worktree agent existing reuse select switch current pane restart replace terminal cwd directory",
        shortcut: "",
        action: CommandAction::RestartPaneInExistingWorktreeWithAgent,
    },
    Command {
        title: "Manage worktrees",
        subtitle: "List this repository's worktrees, see which panes use them, delete unused ones",
        keywords: "git worktree list delete remove clean prune manage existing",
        shortcut: "",
        action: CommandAction::ManageWorktrees,
    },
    Command {
        title: "Sessions",
        subtitle: "Resume a background session, or kill sessions you no longer need",
        keywords: "session resume attach detach background kill list multiplexer",
        shortcut: "",
        action: CommandAction::ManageSessions,
    },
    Command {
        title: "Fleet: show tabs",
        subtitle: "List every pane in tab order in the fleet rail",
        keywords: "fleet view rail sidebar panes tabs all toggle",
        shortcut: "",
        action: CommandAction::FleetTabs,
    },
    Command {
        title: "Fleet: show agents",
        subtitle: "Filter the fleet rail to agent panes only",
        keywords: "fleet view rail sidebar filter agents only toggle",
        shortcut: "",
        action: CommandAction::FleetAgents,
    },
    Command {
        title: "Fleet: group by repository",
        subtitle: "Group every pane by its Git repository",
        keywords: "fleet view rail sidebar panes repos repositories git group toggle",
        shortcut: "",
        action: CommandAction::FleetRepos,
    },
    Command {
        title: "Open GitHub panel",
        subtitle: "Review this pane's repository, changes, and pull request",
        keywords: "git github repository changes files pull request pr merge checks sidebar panel",
        shortcut: "",
        action: CommandAction::OpenGitHubPanel,
    },
    Command {
        title: "Open settings",
        subtitle: "Configure interface and terminal fonts",
        keywords: "preferences font appearance terminal",
        shortcut: "Ctrl/Cmd+,",
        action: CommandAction::OpenSettings,
    },
    Command {
        title: "Launch Codex",
        subtitle: "Start the configured Codex command in a new terminal pane",
        keywords: "agent open run coding",
        shortcut: "",
        action: CommandAction::LaunchAgent(Agent::Codex),
    },
    Command {
        title: "Launch Claude Code",
        subtitle: "Start the configured Claude Code command in a new terminal pane",
        keywords: "agent open run coding claude",
        shortcut: "",
        action: CommandAction::LaunchAgent(Agent::Claude),
    },
    Command {
        title: "Launch Oh My Pi",
        subtitle: "Start the configured Oh My Pi command in a new terminal pane",
        keywords: "agent open run coding pi omp oh my pi",
        shortcut: "",
        action: CommandAction::LaunchAgent(Agent::Pi),
    },
    Command {
        title: "Return to workspace",
        subtitle: "Leave settings and show terminal panes",
        keywords: "terminal back home",
        shortcut: "Esc",
        action: CommandAction::ReturnToWorkspace,
    },
];

pub(crate) fn filtered(query: &str) -> Vec<Command> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect();
    COMMANDS
        .iter()
        .copied()
        .filter(|command| {
            let haystack = format!(
                "{} {} {}",
                command.title, command.subtitle, command.keywords
            )
            .to_ascii_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_search_matches_across_metadata_and_all_terms() {
        let commands = filtered("vertical terminal");
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].action,
            CommandAction::Split(SplitAxis::Vertical)
        );

        let commands = filtered("preferences font");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].action, CommandAction::OpenSettings);
    }

    #[test]
    fn empty_search_returns_the_complete_registry() {
        assert_eq!(filtered("").len(), COMMANDS.len());
    }

    #[test]
    fn agent_commands_are_searchable() {
        let commands = filtered("launch claude");
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].action,
            CommandAction::LaunchAgent(Agent::Claude)
        );
        let commands = filtered("launch pi");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].action, CommandAction::LaunchAgent(Agent::Pi));
    }

    #[test]
    fn repository_grouping_command_is_searchable() {
        let commands = filtered("fleet repository");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].action, CommandAction::FleetRepos);
    }

    #[test]
    fn github_panel_is_searchable_from_the_command_palette() {
        let commands = filtered("github merge checks");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].action, CommandAction::OpenGitHubPanel);
    }

    #[test]
    fn worktree_pane_commands_offer_both_split_directions() {
        let commands = filtered("worktree pane right");
        assert!(commands.iter().any(|command| {
            command.action == CommandAction::NewWorktree(WorktreeKind::Pane(SplitAxis::Horizontal))
        }));

        let commands = filtered("worktree pane down");
        assert!(commands.iter().any(|command| {
            command.action == CommandAction::NewWorktree(WorktreeKind::Pane(SplitAxis::Vertical))
        }));
    }

    #[test]
    fn pane_worktree_restart_commands_distinguish_create_from_reuse() {
        let commands = filtered("create restart pane worktree");
        assert!(
            commands
                .iter()
                .any(|command| command.action == CommandAction::RestartPaneInWorktree)
        );

        let commands = filtered("reuse existing pane worktree");
        assert!(
            commands
                .iter()
                .any(|command| { command.action == CommandAction::RestartPaneInExistingWorktree })
        );
    }

    #[test]
    fn worktree_agent_commands_cover_both_splits_and_current_pane_reuse() {
        let right = filtered("worktree agent pane right");
        assert_eq!(right.len(), 1);
        assert_eq!(
            right[0].action,
            CommandAction::NewWorktreeWithAgent(WorktreeKind::Pane(SplitAxis::Horizontal))
        );

        let down = filtered("worktree agent pane down");
        assert_eq!(down.len(), 1);
        assert_eq!(
            down[0].action,
            CommandAction::NewWorktreeWithAgent(WorktreeKind::Pane(SplitAxis::Vertical))
        );

        let current = filtered("existing worktree agent current pane");
        assert_eq!(current.len(), 1);
        assert_eq!(
            current[0].action,
            CommandAction::RestartPaneInExistingWorktreeWithAgent
        );
    }

    #[test]
    fn tiled_pane_commands_are_identified_for_maximized_mode() {
        assert!(CommandAction::Split(SplitAxis::Horizontal).requires_tiled_panes());
        assert!(CommandAction::GrowPane.requires_tiled_panes());
        assert!(CommandAction::RestorePaneSize.requires_tiled_panes());
        assert!(CommandAction::PreviousPaneLayout.requires_tiled_panes());
        assert!(CommandAction::NextPaneLayout.requires_tiled_panes());
        assert!(
            CommandAction::NewWorktree(WorktreeKind::Pane(SplitAxis::Vertical))
                .requires_tiled_panes()
        );
        assert!(
            CommandAction::NewWorktreeWithAgent(WorktreeKind::Pane(SplitAxis::Vertical))
                .requires_tiled_panes()
        );
        assert!(CommandAction::LaunchAgent(Agent::Codex).requires_tiled_panes());

        assert!(!CommandAction::ClosePane.requires_tiled_panes());
        assert!(!CommandAction::NewTab.requires_tiled_panes());
        assert!(!CommandAction::RestartPaneInWorktree.requires_tiled_panes());
        assert!(!CommandAction::OpenSettings.requires_tiled_panes());
    }
}
