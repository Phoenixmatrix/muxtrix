//! Unit tests for the application core.
//!
//! These drive `Muxtrix` through real messages and assert on the state and the
//! effects that come back, so they exercise the same path the running app
//! does without needing a window.

use super::*;

#[test]
fn settings_change_detection_tracks_edits_and_reverts() {
    let saved = AppSettings::default();
    let mut draft = saved.clone();

    assert!(!settings_have_changes(&saved, &draft));
    draft.show_status_bar = !draft.show_status_bar;
    assert!(settings_have_changes(&saved, &draft));
    draft.show_status_bar = saved.show_status_bar;
    assert!(!settings_have_changes(&saved, &draft));
}

#[test]
fn scrollback_history_input_updates_the_draft_with_an_arbitrary_bounded_value() {
    let mut app = Muxtrix::new();
    drop(app.open_settings());

    let _ = app.update(Message::SettingsScrollbackLimit("42,731".into()));

    assert_eq!(app.settings_scrollback_lines_input, "42,731");
    assert_eq!(app.settings_draft.terminal_scrollback_lines, 42_731);
    assert_eq!(
        app.settings.terminal_scrollback_lines,
        settings::DEFAULT_TERMINAL_SCROLLBACK_LINES
    );
}

#[test]
fn invalid_scrollback_history_input_cannot_be_saved() {
    let mut app = Muxtrix::new();
    app.active_view = ActiveView::Settings;
    app.settings_scrollback_lines_input = "999".into();

    drop(app.save_settings());

    assert_eq!(app.active_view, ActiveView::Settings);
    assert_eq!(
        app.settings.terminal_scrollback_lines,
        settings::DEFAULT_TERMINAL_SCROLLBACK_LINES
    );
    assert!(app.status.contains("between 1,000 and 100,000 lines"));
}

#[test]
fn workspace_visibility_setting_is_drafted_until_applied() {
    let mut app = Muxtrix::new();

    let _ = app.update(Message::SettingsShowAllWorkspaces(true));

    assert_eq!(app.settings.fleet_scope, FleetScope::CurrentWorkspace);
    assert_eq!(app.settings_draft.fleet_scope, FleetScope::AllWorkspaces);
    assert!(settings_have_changes(&app.settings, &app.settings_draft));

    let _ = app.update(Message::CancelSettings);
    assert_eq!(app.settings_draft.fleet_scope, FleetScope::CurrentWorkspace);
    assert!(!settings_have_changes(&app.settings, &app.settings_draft));
}

#[test]
fn binary_version_response_requires_the_expected_name_and_one_version() {
    assert_eq!(
        parse_binary_version("muxtrix 1.2.3\n", "muxtrix"),
        Ok("1.2.3".into())
    );
    assert!(parse_binary_version("muxtrixctl 1.2.3", "muxtrix").is_err());
    assert!(parse_binary_version("muxtrix 1.2.3 extra", "muxtrix").is_err());
}

#[test]
fn startup_path_preserves_the_invocation_location_without_canonicalizing() {
    let current_directory = std::path::Path::new("launch-root");
    let resolved = resolve_startup_executable(
        std::ffi::OsStr::new("installed/bin/muxtrix"),
        current_directory,
        None,
    )
    .expect("relative invocation path should resolve from the startup directory");
    assert_eq!(
        resolved,
        current_directory.join("installed/bin/muxtrix"),
        "the saved path must keep pointing at the package-managed entry after its target changes"
    );
}

#[test]
fn startup_offers_resumable_sessions_before_spawning_a_daemon() {
    let session_id = uuid::Uuid::new_v4();
    let candidate = muxtrix_sessions::SessionRecord {
        id: session_id,
        name: "existing".into(),
        endpoint: "existing-session".into(),
        process_id: 42,
        created_unix: 1,
        layout: Some("{}".into()),
        attached: false,
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let started = std::cell::Cell::new(false);
    let offered = start_host_unless_resumable(vec![candidate], || {
        started.set(true);
        Ok(())
    })
    .expect("startup decision should succeed");

    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].id, session_id);
    assert!(
        !started.get(),
        "discovering a resumable session must not create a throwaway daemon"
    );

    let started = std::cell::Cell::new(false);
    let offered = start_host_unless_resumable(Vec::new(), || {
        started.set(true);
        Ok(())
    })
    .expect("fresh session startup should succeed");
    assert!(offered.is_empty());
    assert!(
        started.get(),
        "an explicit fresh start must create its daemon"
    );
}

#[test]
fn installed_version_mismatch_requests_a_restart() {
    let matching = InstalledVersionsState::Ready(InstalledVersions {
        muxtrix: Ok(env!("CARGO_PKG_VERSION").into()),
        muxtrixctl: Ok(muxtrix_control::VERSION.into()),
    });
    assert_eq!(installed_version_restart_copy(&matching), None);

    let installed = format!("{}-installed", env!("CARGO_PKG_VERSION"));
    let mismatch = InstalledVersionsState::Ready(InstalledVersions {
        muxtrix: Ok(installed.clone()),
        muxtrixctl: Ok(muxtrix_control::VERSION.into()),
    });
    let notice =
        installed_version_restart_copy(&mismatch).expect("mismatch should request a restart");
    assert!(notice.contains(&installed));
    assert!(notice.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn github_diff_window_bounds_large_files_with_overscan() {
    let line_starts = (0..=50_000).collect::<Vec<_>>();
    assert_eq!(
        github_diff_window(&line_starts, 0.0, 480.0),
        (0, 36, 0, 49_964)
    );
    let (first, last, top_rows, bottom_rows) = github_diff_window(&line_starts, 24_000.0, 480.0);
    assert_eq!(
        (first, last, top_rows, bottom_rows),
        (984, 1_036, 984, 48_964)
    );
    assert_eq!(
        github_diff_window(&[0, 1, 2, 3], 9_999.0, 480.0),
        (3, 3, 3, 0)
    );
}

#[test]
fn github_diff_wraps_only_when_the_code_lane_holds_eighty_cells() {
    let threshold =
        GITHUB_PANEL_WIDTH + GITHUB_DIFF_CHROME_WIDTH + GITHUB_DIFF_MIN_WRAP_COLUMNS as f32 * 8.0;
    assert_eq!(github_diff_wrap_columns(threshold - 1.0, 8.0), None);
    assert_eq!(
        github_diff_wrap_columns(threshold, 8.0),
        Some(GITHUB_DIFF_MIN_WRAP_COLUMNS)
    );
}

#[test]
fn github_diff_layout_counts_wrapped_visual_rows_without_repeating_gutters() {
    let document = github::DiffDocument {
        lines: vec![github::DiffLine {
            kind: github::DiffLineKind::Addition,
            old_line: None,
            new_line: Some(12),
            text: format!("+{}", "x".repeat(160)),
        }],
        notice: None,
        truncated: false,
        max_columns: 161,
    };
    let starts = github_diff_line_starts(&document, Some(80));

    assert_eq!(starts, vec![0, 3]);
    assert_eq!(github_diff_line_for_visual_row(&starts, 0), 0);
    assert_eq!(github_diff_line_for_visual_row(&starts, 2), 0);
}

#[test]
fn stale_github_authentication_results_cannot_replace_the_configured_host_state() {
    let mut app = Muxtrix::new();
    app.github_auth_generation = 2;
    app.github_auth = github::AuthStatus::Checking;

    drop(app.update(Message::GitHubAuthChecked(
        1,
        github::AuthStatus::Authenticated {
            login: "stale-account".into(),
        },
    )));
    assert_eq!(app.github_auth, github::AuthStatus::Checking);

    drop(app.update(Message::GitHubAuthChecked(
        2,
        github::AuthStatus::Authenticated {
            login: "enterprise-account".into(),
        },
    )));
    assert_eq!(
        app.github_auth,
        github::AuthStatus::Authenticated {
            login: "enterprise-account".into()
        }
    );
}
#[test]
fn startup_authentication_starts_the_visible_pull_request_list() {
    let mut app = Muxtrix::new();
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.loading = false;
    app.github_panel = Some(panel);
    app.github_auth_generation = 3;

    drop(app.update(Message::GitHubAuthChecked(
        3,
        github::AuthStatus::Authenticated {
            login: "octocat".into(),
        },
    )));

    assert!(
        app.github_panel
            .as_ref()
            .is_some_and(|panel| panel.pull_requests_loading)
    );
}

#[test]
fn invalid_github_host_keeps_the_settings_draft_open() {
    let mut app = Muxtrix::new();
    app.active_view = ActiveView::Settings;
    app.settings_draft.github_host = "github.example.com/api/v3".into();

    drop(app.save_settings());

    assert_eq!(app.active_view, ActiveView::Settings);
    assert_eq!(app.settings_draft.github_host, "github.example.com/api/v3");
    assert!(app.status.contains("GitHub host must be a hostname"));
}

#[test]
fn opening_github_panel_defers_repository_discovery() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "codex".into(),
            display_name: None,
            state: AgentState::Idle,
            activity: None,
            session_id: None,
            cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            git_branch: None,
        },
    );

    drop(app.open_github_panel());

    let panel = app
        .github_panel
        .as_ref()
        .expect("pending panel should open");
    assert!(panel.context_loading);
    assert!(panel.data.is_none());
    assert!(panel.repository.name.is_empty());
    assert_eq!(app.github_context_generation, 1);
}

#[test]
fn loaded_initial_github_context_opens_panel() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let root = std::env::temp_dir().join("muxtrix");
    app.github_context_generation = 1;
    let mut pending = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: String::new(),
        owner_and_name: None,
        host: "github.com".into(),
        branch: String::new(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    pending.context_loading = true;
    app.github_panel = Some(pending);

    drop(app.update(Message::GitHubFocusedPaneLoaded(
        pane_id,
        1,
        GitHubContextLoad::Open,
        Box::new(Ok((
            github::Repository {
                root: root.clone(),
                name: "muxtrix".into(),
                owner_and_name: Some("example/muxtrix".into()),
                host: "github.com".into(),
                branch: "main".into(),
                head_oid: String::new(),
                wsl_distribution: String::new(),
            },
            github::PanelData {
                branch: "main".into(),
                files: Vec::new(),
                additions: 0,
                deletions: 0,
                current_pull_request: None,
            },
        ))),
    )));

    let panel = app.github_panel.as_ref().expect("panel should open");
    assert_eq!(panel.repository.root, root);
    assert_eq!(
        panel.data.as_ref().map(|data| data.branch.as_str()),
        Some("main")
    );
    assert!(!panel.loading);
}

#[test]
fn closed_github_panel_ignores_refresh_results() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.github_context_generation = 1;
    app.status = "Workspace ready".into();

    drop(app.update(Message::GitHubFocusedPaneLoaded(
        pane_id,
        1,
        GitHubContextLoad::Refresh,
        Box::new(Err("late refresh failure".into())),
    )));

    assert!(app.github_panel.is_none());
    assert_eq!(app.status, "Workspace ready");
}

#[test]
fn closed_pending_github_panel_ignores_open_result() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.github_context_generation = 1;
    app.status = "Workspace ready".into();

    drop(app.update(Message::GitHubFocusedPaneLoaded(
        pane_id,
        1,
        GitHubContextLoad::Open,
        Box::new(Err("late opening failure".into())),
    )));

    assert!(app.github_panel.is_none());
    assert_eq!(app.status, "Workspace ready");
}

#[test]
fn selecting_a_github_file_opens_the_diff_and_back_restores_the_workspace() {
    let mut app = Muxtrix::new();
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "diff-viewer".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.data = Some(github::PanelData {
        branch: "diff-viewer".into(),
        files: vec![github::FileChange {
            path: "src/main.rs".into(),
            previous_path: None,
            status: "Modified".into(),
            additions: 1,
            deletions: 1,
            patch: Some("@@ -1 +1 @@\n-old\n+new".into()),
        }],
        additions: 1,
        deletions: 1,
        current_pull_request: None,
    });
    panel.loading = false;
    app.github_panel = Some(panel);

    drop(app.update(Message::OpenGitHubDiff("src/main.rs".into())));
    assert_eq!(app.active_view, ActiveView::GitHubDiff);
    assert_eq!(
        app.github_diff.as_ref().map(|diff| diff.path.as_str()),
        Some("src/main.rs")
    );
    assert_eq!(
        app.github_diff.as_ref().map(|diff| diff.source),
        Some(GitHubDiffSource::Local)
    );

    drop(app.update(Message::CloseGitHubDiff));
    assert_eq!(app.active_view, ActiveView::Workspace);
    assert!(app.github_diff.is_none());
    assert!(app.github_panel.is_some());
}

#[test]
fn github_panel_open_publishes_loading_state_before_repository_probe() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let directory = std::env::temp_dir().join("github-panel-loading");
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "codex".into(),
            display_name: None,
            state: AgentState::Running,
            activity: None,
            session_id: None,
            cwd: Some(directory.display().to_string()),
            git_branch: Some("main".into()),
        },
    );

    drop(app.open_github_panel());

    assert!(app.github_panel.as_ref().is_some_and(|panel| {
        panel.repository.root == directory
            && panel.repository.name.is_empty()
            && panel.context_loading
    }));
}

#[test]
fn periodic_repository_refresh_does_not_restart_an_in_flight_probe() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let directory = app
        .pane_working_directory(pane_id)
        .expect("active pane should have a working directory");
    app.pending_repository_directories
        .insert(pane_id, directory.clone());
    app.pane_repository_generation = 7;

    drop(app.refresh_pane_repositories());

    assert_eq!(app.pane_repository_generation, 7);
    assert_eq!(
        app.pending_repository_directories.get(&pane_id),
        Some(&directory)
    );
}

#[test]
fn stale_repository_metadata_is_reprobed_without_a_directory_or_branch_change() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let directory = app
        .pane_working_directory(pane_id)
        .expect("active pane should have a working directory");
    let current = github::CurrentPullRequest {
        number: 173,
        url: "https://github.com/example/muxtrix/pull/173".into(),
        state: github::CurrentPullRequestState::Open,
    };
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory: directory.clone(),
            root: Some(directory.clone()),
            name: Some("muxtrix".into()),
            worktree_name: None,
            branch: Some("main".into()),
            reported_branch: None,
            head_oid: Some("old-head".into()),
            pull_request: Some(current.clone()),
            checked_at: std::time::Instant::now() - PANE_REPOSITORY_INTERVAL,
        },
    );

    drop(app.refresh_pane_repositories());

    assert_eq!(
        app.pending_repository_directories.get(&pane_id),
        Some(&directory)
    );
    assert_eq!(app.pane_repository_generation, 1);
    assert_eq!(
        app.pane_repositories
            .get(&pane_id)
            .and_then(|repository| repository.pull_request.as_ref()),
        Some(&current),
        "the last confirmed PR stays visible while its refresh is pending"
    );
}

#[test]
fn failed_pr_refresh_keeps_the_last_confirmed_indicator() {
    let current = github::CurrentPullRequest {
        number: 173,
        url: "https://github.com/example/muxtrix/pull/173".into(),
        state: github::CurrentPullRequestState::Open,
    };

    assert_eq!(
        current_pull_request_after_refresh(
            Some(current.clone()),
            Err("temporary GitHub failure".into()),
        ),
        Some(current)
    );
    assert_eq!(
        current_pull_request_after_refresh(
            Some(github::CurrentPullRequest {
                number: 173,
                url: "https://github.com/example/muxtrix/pull/173".into(),
                state: github::CurrentPullRequestState::Open,
            }),
            Ok(None),
        ),
        None,
        "a successful no-PR answer clears the indicator"
    );
}

/// A probe that cannot confirm the branch the hook reported — an
/// unreachable checkout, a detached HEAD, a git call that failed — used to
/// leave the entry permanently stale. This refresh runs on every terminal
/// event, so that re-probed without pause and spent six console
/// subprocesses per pane on each pass; on Windows every one of those is a
/// `conhost.exe`.
#[test]
fn a_branch_the_probe_could_not_confirm_does_not_reprobe_forever() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let directory = app
        .pane_working_directory(pane_id)
        .expect("active pane should have a working directory");
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Running,
            activity: None,
            session_id: None,
            cwd: None,
            git_branch: Some("feature".into()),
        },
    );
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory: directory.clone(),
            root: None,
            name: None,
            worktree_name: None,
            branch: None,
            reported_branch: Some("feature".into()),
            head_oid: None,
            pull_request: None,
            checked_at: std::time::Instant::now(),
        },
    );

    drop(app.refresh_pane_repositories());

    assert!(app.pending_repository_directories.is_empty());
    assert_eq!(app.pane_repository_generation, 0);
}

#[test]
fn a_newly_reported_branch_invalidates_the_cached_repository() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let directory = app
        .pane_working_directory(pane_id)
        .expect("active pane should have a working directory");
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Running,
            activity: None,
            session_id: None,
            cwd: None,
            git_branch: Some("feature".into()),
        },
    );
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory: directory.clone(),
            root: Some(directory.clone()),
            name: Some("muxtrix".into()),
            worktree_name: None,
            branch: Some("main".into()),
            reported_branch: Some("main".into()),
            head_oid: Some("old-head".into()),
            pull_request: Some(github::CurrentPullRequest {
                number: 173,
                url: "https://github.com/example/muxtrix/pull/173".into(),
                state: github::CurrentPullRequestState::Open,
            }),
            checked_at: std::time::Instant::now(),
        },
    );

    drop(app.refresh_pane_repositories());

    assert_eq!(
        app.pending_repository_directories.get(&pane_id),
        Some(&directory)
    );
    assert_eq!(app.pane_repository_generation, 1);
    assert!(
        app.pane_repositories
            .get(&pane_id)
            .is_none_or(|repository| repository.pull_request.is_none()),
        "the prior branch's PR must disappear before the new probe lands"
    );
}

#[test]
fn completed_turn_refresh_keeps_confirmed_pr_when_branch_report_is_missing() {
    let mut app = Muxtrix::new();
    let original = active_pane_id(&app);
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane should open");
    let directory = app
        .pane_working_directory(original)
        .expect("original pane should have a working directory");
    let current = github::CurrentPullRequest {
        number: 173,
        url: "https://github.com/example/muxtrix/pull/173".into(),
        state: github::CurrentPullRequestState::Open,
    };
    app.pane_repositories.insert(
        original,
        PaneRepository {
            directory: directory.clone(),
            root: Some(directory),
            name: Some("muxtrix".into()),
            worktree_name: None,
            branch: Some("feature".into()),
            reported_branch: Some("feature".into()),
            head_oid: Some("old-head".into()),
            pull_request: Some(current.clone()),
            checked_at: std::time::Instant::now(),
        },
    );

    app.queue_github_pull_request_refresh(original);
    drop(app.refresh_pane_repositories());

    let cached = app
        .pane_repositories
        .get(&original)
        .expect("completed turn should preserve cached repository metadata");
    assert_eq!(cached.pull_request.as_ref(), Some(&current));
    assert!(
        cached.checked_at.elapsed() >= PANE_REPOSITORY_INTERVAL,
        "completed turn should still make the repository due for refresh"
    );
}

#[test]
fn changing_worktrees_clears_the_stale_pr_before_the_probe_lands() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let directory = app
        .pane_working_directory(pane_id)
        .expect("active pane should have a working directory");
    let other_worktree = directory.with_file_name("mk-173-other-worktree");
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory: directory.clone(),
            root: Some(directory),
            name: Some("muxtrix".into()),
            worktree_name: None,
            branch: Some("main".into()),
            reported_branch: Some("main".into()),
            head_oid: Some("old-head".into()),
            pull_request: Some(github::CurrentPullRequest {
                number: 172,
                url: "https://github.com/example/muxtrix/pull/172".into(),
                state: github::CurrentPullRequestState::Open,
            }),
            checked_at: std::time::Instant::now(),
        },
    );
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Running,
            activity: None,
            session_id: None,
            cwd: Some(other_worktree.display().to_string()),
            git_branch: Some("main".into()),
        },
    );

    drop(app.refresh_pane_repositories());

    assert!(
        app.pane_repositories
            .get(&pane_id)
            .is_none_or(|repository| repository.pull_request.is_none())
    );
    assert_eq!(
        app.pending_repository_directories.get(&pane_id),
        Some(&other_worktree)
    );
}

#[test]
fn pane_focus_invalidates_local_diff_and_wakes_refresh_subscription() {
    let mut app = Muxtrix::new();
    let original = active_pane_id(&app);
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane should open");
    let second = active_pane_id(&app);
    assert_ne!(original, second);
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.context_loading = false;
    panel.loading = false;
    app.github_panel = Some(panel);
    app.github_pane_refresh_pending = false;
    while app.event_receiver.try_recv().is_ok() {}

    app.focus_pane(original)
        .expect("original pane should focus");

    assert!(app.github_pane_refresh_pending);
    assert!(
        app.github_panel
            .as_ref()
            .is_some_and(|panel| panel.context_loading)
    );
    app.event_receiver
        .try_recv()
        .expect("focus change should wake the stable app subscription");
}

#[test]
fn workspace_switch_invalidates_local_diff_and_wakes_refresh_subscription() {
    let mut app = Muxtrix::new();
    let first_workspace = app.session.active_workspace_id;
    let first_pane = active_pane_id(&app);
    create_test_workspace(&mut app);
    assert_ne!(first_workspace, app.session.active_workspace_id);
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.context_loading = false;
    panel.loading = false;
    app.github_panel = Some(panel);
    app.github_pane_refresh_pending = false;
    while app.event_receiver.try_recv().is_ok() {}

    app.switch_workspace(first_workspace)
        .expect("first workspace should activate");

    assert_eq!(active_pane_id(&app), first_pane);
    assert!(app.github_pane_refresh_pending);
    assert!(
        app.github_panel
            .as_ref()
            .is_some_and(|panel| panel.context_loading)
    );
    app.event_receiver
        .try_recv()
        .expect("workspace switch should wake the stable app subscription");
}

#[test]
fn stale_local_diff_result_cannot_overwrite_newly_focused_pane() {
    let mut app = Muxtrix::new();
    let original = active_pane_id(&app);
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane should open");
    let second = active_pane_id(&app);
    assert_ne!(original, second);
    let current_root = std::env::temp_dir();
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: current_root.clone(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.context_loading = false;
    panel.loading = false;
    app.github_panel = Some(panel);
    app.github_context_generation = 7;
    app.github_pane_refresh_pending = false;

    let stale_root = current_root.join("stale-pane");
    drop(app.update(Message::GitHubFocusedPaneLoaded(
        original,
        7,
        GitHubContextLoad::Refresh,
        Box::new(Ok((
            github::Repository {
                root: stale_root,
                name: "stale".into(),
                owner_and_name: Some("example/stale".into()),
                host: "github.com".into(),
                branch: "stale".into(),
                head_oid: String::new(),
                wsl_distribution: String::new(),
            },
            github::PanelData {
                branch: "stale".into(),
                files: Vec::new(),
                additions: 0,
                deletions: 0,
                current_pull_request: None,
            },
        ))),
    )));

    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert_eq!(active_pane_id(&app), second);
    assert_eq!(panel.repository.root, current_root);
    assert!(panel.context_loading);
    assert_eq!(
        app.github_context_generation, 8,
        "the focus mismatch must launch a fresh request instead of waiting for a timer"
    );
}

#[test]
fn local_refresh_clamps_file_cursor_after_the_change_set_shrinks() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let root = std::env::temp_dir().join("github-local-clamp");
    let repository = github::Repository {
        root: root.clone(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    };
    let file = github::FileChange {
        path: "src/main.rs".into(),
        previous_path: None,
        status: "Modified".into(),
        additions: 1,
        deletions: 0,
        patch: None,
    };
    let mut panel = GitHubPanelState::loading(repository.clone());
    panel.loading = false;
    panel.context_loading = true;
    panel.data = Some(github::PanelData {
        branch: "main".into(),
        files: vec![file.clone(); 5],
        additions: 5,
        deletions: 0,
        current_pull_request: None,
    });
    panel.file_keyboard_cursor = Some(4);
    panel.file_scroll_offset = 9_999.0;
    app.github_panel = Some(panel);
    app.github_context_generation = 3;

    drop(app.update(Message::GitHubFocusedPaneLoaded(
        pane_id,
        3,
        GitHubContextLoad::Refresh,
        Box::new(Ok((
            repository,
            github::PanelData {
                branch: "main".into(),
                files: vec![file],
                additions: 1,
                deletions: 0,
                current_pull_request: None,
            },
        ))),
    )));

    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert_eq!(panel.file_keyboard_cursor, Some(0));
    assert_eq!(panel.file_scroll_offset, 0.0);
}

#[test]
fn harness_turn_completion_events_exclude_pi_maintenance() {
    assert!(agent_event_completes_turn(
        AgentState::Completed,
        Some("Stop")
    ));
    assert!(agent_event_completes_turn(
        AgentState::Completed,
        Some("agent_end")
    ));
    assert!(!agent_event_completes_turn(
        AgentState::Completed,
        Some("session_compact")
    ));
    assert!(!agent_event_completes_turn(
        AgentState::Completed,
        Some("auto_compaction_end")
    ));
    assert!(!agent_event_completes_turn(
        AgentState::Running,
        Some("agent_end")
    ));
}

#[test]
fn completed_agent_turn_queues_visible_pull_request_refresh() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.github_auth = github::AuthStatus::Authenticated {
        login: "octocat".into(),
    };
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "mk-152".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.loading = false;
    panel.pull_requests = Some(Vec::new());
    app.github_panel = Some(panel);

    let response = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Completed,
        event: Some("Stop".into()),
        title: "Codex · Stop".into(),
        body: "Agent completed a turn".into(),
        pane_id: Some(pane_id.as_uuid().to_string()),
        session_id: Some("mk-152-session".into()),
        cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
    });

    assert!(response.ok);
    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert!(panel.pull_requests.is_none());
    assert!(panel.pull_requests_loading);
    assert!(app.github_pull_requests_refresh_pending);

    drop(app.update(Message::RefreshGitHubPullRequestsAfterAgentTurn));
    assert!(!app.github_pull_requests_refresh_pending);
    assert_eq!(
        app.github_panel
            .as_ref()
            .map(|panel| panel.pull_request_generation),
        Some(1)
    );
}

#[test]
fn completed_agent_turn_refreshes_the_selected_pull_request_and_list() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.github_auth = github::AuthStatus::Authenticated {
        login: "octocat".into(),
    };
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "mk-152".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.loading = false;
    panel.pull_requests = Some(Vec::new());
    panel.selected_pull_request_number = Some(42);
    panel.selected_pull_request_file_scroll_offset = 84.0;
    panel.file_keyboard_cursor = Some(3);
    panel.keyboard_focus = Some(GitHubPanelKeyboardFocus::Files);
    app.github_panel = Some(panel);

    let response = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Completed,
        event: Some("Stop".into()),
        title: "Codex · Stop".into(),
        body: "Agent completed a turn".into(),
        pane_id: Some(pane_id.as_uuid().to_string()),
        session_id: Some("mk-152-session".into()),
        cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
    });

    assert!(response.ok);
    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert!(panel.pull_requests.is_none());
    assert!(panel.pull_requests_loading);
    assert!(panel.selected_pull_request_loading);
    assert!(app.github_pull_requests_refresh_pending);

    drop(app.update(Message::RefreshGitHubPullRequestsAfterAgentTurn));
    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert!(!app.github_pull_requests_refresh_pending);
    assert_eq!(panel.pull_request_generation, 1);
    assert_eq!(panel.pull_request_detail_generation, 2);
    assert_eq!(panel.selected_pull_request_file_scroll_offset, 84.0);
    assert_eq!(panel.file_keyboard_cursor, Some(3));
    assert_eq!(panel.keyboard_focus, Some(GitHubPanelKeyboardFocus::Files));
}

#[test]
fn completed_agent_turn_invalidates_hidden_pull_request_cache() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "mk-152".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.loading = false;
    panel.pull_requests = Some(Vec::new());
    app.github_panel = Some(panel);

    let response = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "pi".into(),
        state: AgentState::Completed,
        event: Some("agent_end".into()),
        title: "Oh My Pi".into(),
        body: "Agent completed a turn".into(),
        pane_id: Some(pane_id.as_uuid().to_string()),
        session_id: Some("mk-152-session".into()),
        cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
    });

    assert!(response.ok);
    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert!(panel.pull_requests.is_none());
    assert!(!panel.pull_requests_loading);
    assert!(!app.github_pull_requests_refresh_pending);
}

#[test]
fn github_pull_request_list_supports_arrow_and_enter_navigation() {
    let mut app = Muxtrix::new();
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.keyboard_focus = Some(GitHubPanelKeyboardFocus::PullRequestList);
    panel.loading = false;
    panel.pull_requests = Some(vec![github::PullRequestSummary {
        number: 42,
        title: "Keyboard-safe ledger".into(),
        url: "https://github.com/example/muxtrix/pull/42".into(),
        author: "octocat".into(),
        head: "keyboard-ledger".into(),
        base: "main".into(),
        status: github::PullRequestSummaryStatus::Open,
        readiness: github::MergeReadiness::Ready,
    }]);
    app.github_panel = Some(panel);

    drop(app.handle_keyboard(key_press(Key::Named(Named::ArrowDown), Modifiers::empty())));
    assert_eq!(
        app.github_panel
            .as_ref()
            .and_then(|panel| panel.pull_request_keyboard_cursor),
        Some(0)
    );

    drop(app.handle_keyboard(key_press(Key::Named(Named::Enter), Modifiers::empty())));
    assert_eq!(
        app.github_panel
            .as_ref()
            .and_then(|panel| panel.selected_pull_request_number),
        Some(42)
    );
}

#[test]
fn pull_request_conflicts_use_error_icon_and_explanation() {
    let pull_request = github::PullRequestSummary {
        number: 42,
        title: "Conflicting pull request".into(),
        url: "https://github.com/example/muxtrix/pull/42".into(),
        author: "octocat".into(),
        head: "conflicts".into(),
        base: "main".into(),
        status: github::PullRequestSummaryStatus::Open,
        readiness: github::MergeReadiness::Conflicts,
    };
    let tokens = DesignTokens::for_appearance(Appearance::Dark);

    assert!(matches!(
        github_readiness_icon(pull_request.readiness),
        IconKind::StatusError
    ));
    let (label, detail, color) = github_pull_request_summary_copy(&pull_request, tokens);
    assert_eq!(label, "Merge conflicts");
    assert_eq!(detail, "Resolve conflicts before merging");
    assert_eq!(color, tokens.danger);
}

#[test]
fn pull_request_actions_update_panel_and_fleet_marker_caches() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let root = std::env::temp_dir().join("github-action-cache");
    let current = github::CurrentPullRequest {
        number: 42,
        url: "https://github.com/example/muxtrix/pull/42".into(),
        state: github::CurrentPullRequestState::Draft,
    };
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: root.clone(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "feature".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.loading = false;
    panel.data = Some(github::PanelData {
        branch: "feature".into(),
        files: Vec::new(),
        additions: 0,
        deletions: 0,
        current_pull_request: Some(current.clone()),
    });
    app.github_panel = Some(panel);
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory: root.clone(),
            root: Some(root.clone()),
            name: Some("muxtrix".into()),
            worktree_name: None,
            branch: Some("feature".into()),
            reported_branch: None,
            head_oid: Some(String::new()),
            pull_request: Some(current),
            checked_at: std::time::Instant::now(),
        },
    );

    drop(app.update(Message::GitHubPullRequestDraftChanged(
        root.clone(),
        42,
        0,
        false,
        Ok("ready".into()),
    )));
    assert_eq!(
        app.pane_repositories[&pane_id]
            .pull_request
            .as_ref()
            .map(|pull_request| pull_request.state),
        Some(github::CurrentPullRequestState::Open)
    );
    assert_eq!(
        app.github_panel
            .as_ref()
            .and_then(|panel| panel.data.as_ref())
            .and_then(|data| data.current_pull_request.as_ref())
            .map(|pull_request| pull_request.state),
        Some(github::CurrentPullRequestState::Open)
    );

    drop(app.update(Message::GitHubMergeFinished(
        root,
        42,
        0,
        Ok("merged".into()),
    )));
    assert_eq!(
        app.pane_repositories[&pane_id]
            .pull_request
            .as_ref()
            .map(|pull_request| pull_request.state),
        Some(github::CurrentPullRequestState::Merged)
    );
}

#[test]
fn draft_update_keeps_detail_and_summary_in_sync() {
    let mut app = Muxtrix::new();
    let repository = github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "draft-toggle".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    };
    let mut panel = GitHubPanelState::loading(repository.clone());
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.loading = false;
    panel.pull_requests = Some(vec![github::PullRequestSummary {
        number: 42,
        title: "Draft completion flow".into(),
        url: "https://github.com/example/muxtrix/pull/42".into(),
        author: "octocat".into(),
        head: "draft-toggle".into(),
        base: "main".into(),
        status: github::PullRequestSummaryStatus::Draft,
        readiness: github::MergeReadiness::Draft,
    }]);
    panel.selected_pull_request_number = Some(42);
    panel.selected_pull_request = Some(github::PullRequestDetails {
        pull_request: github::PullRequest {
            number: 42,
            title: "Draft completion flow".into(),
            url: "https://github.com/example/muxtrix/pull/42".into(),
            author: "octocat".into(),
            head: "draft-toggle".into(),
            head_oid: "deadbeef".into(),
            head_repository: "example/muxtrix".into(),
            base: "main".into(),
            base_oid: "feedface".into(),
            additions: 1,
            deletions: 0,
            changed_files: 0,
            draft: true,
            mergeable: "MERGEABLE".into(),
            merge_state: "CLEAN".into(),
            review_decision: "APPROVED".into(),
            checks: github::CheckSummary {
                passed: 1,
                pending: 0,
                failed: 0,
            },
        },
        files: Vec::new(),
    });
    panel.draft_state_updating = true;
    panel.keyboard_focus = Some(GitHubPanelKeyboardFocus::Back);
    assert_eq!(
        github_keyboard_focus_step(&panel, GitHubPanelKeyboardFocus::Back, true),
        GitHubPanelKeyboardFocus::DraftAction
    );
    assert_eq!(
        github_keyboard_focus_step(&panel, GitHubPanelKeyboardFocus::DraftAction, true),
        GitHubPanelKeyboardFocus::Files,
        "draft pull requests must not expose the disabled merge action"
    );
    app.github_panel = Some(panel);
    drop(app.handle_keyboard(key_press(Key::Named(Named::Escape), Modifiers::empty())));
    assert_eq!(
        app.github_panel
            .as_ref()
            .and_then(|panel| panel.selected_pull_request_number),
        Some(42),
        "an in-flight draft update must keep its detail context"
    );

    drop(app.update(Message::GitHubPullRequestDraftChanged(
        repository.root,
        42,
        0,
        false,
        Ok("Marked pull request #42 ready for review".into()),
    )));

    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert!(!panel.draft_state_updating);
    assert!(
        !panel
            .selected_pull_request
            .as_ref()
            .expect("detail should remain visible")
            .pull_request
            .draft
    );
    assert_eq!(
        panel
            .pull_requests
            .as_ref()
            .expect("summary should remain loaded")[0]
            .status,
        github::PullRequestSummaryStatus::Open
    );
    assert_eq!(
        github_keyboard_focus_step(panel, GitHubPanelKeyboardFocus::DraftAction, true),
        GitHubPanelKeyboardFocus::MergeAction
    );
}

#[test]
fn merged_pull_request_returns_to_list_and_refreshes_optimistically() {
    let mut app = Muxtrix::new();
    let repository = github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    };
    let mut panel = GitHubPanelState::loading(repository.clone());
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.loading = false;
    panel.pull_requests = Some(vec![github::PullRequestSummary {
        number: 42,
        title: "Merge completion flow".into(),
        url: "https://github.com/example/muxtrix/pull/42".into(),
        author: "octocat".into(),
        head: "merge-flow".into(),
        base: "main".into(),
        status: github::PullRequestSummaryStatus::Open,
        readiness: github::MergeReadiness::Ready,
    }]);
    panel.selected_pull_request_number = Some(42);
    panel.selected_pull_request = Some(github::PullRequestDetails {
        pull_request: github::PullRequest {
            number: 42,
            title: "Merge completion flow".into(),
            url: "https://github.com/example/muxtrix/pull/42".into(),
            author: "octocat".into(),
            head: "merge-flow".into(),
            head_oid: "deadbeef".into(),
            head_repository: "example/muxtrix".into(),
            base: "main".into(),
            base_oid: "feedface".into(),
            additions: 1,
            deletions: 0,
            changed_files: 0,
            draft: false,
            mergeable: "MERGEABLE".into(),
            merge_state: "CLEAN".into(),
            review_decision: "APPROVED".into(),
            checks: github::CheckSummary {
                passed: 1,
                pending: 0,
                failed: 0,
            },
        },
        files: Vec::new(),
    });
    panel.merging = true;
    app.github_auth = github::AuthStatus::Authenticated {
        login: "octocat".into(),
    };
    app.github_panel = Some(panel);
    app.github_diff = Some(GitHubDiffState {
        source: GitHubDiffSource::PullRequest(42),
        path: "src/main.rs".into(),
        status: "Modified".into(),
        additions: 1,
        deletions: 0,
        document: None,
        loading: false,
        error: None,
        generation: 0,
        cancellation: ProcessCancellation::default(),
        scroll_offset: 0.0,
        wrap_columns: None,
        line_starts: Vec::new(),
    });
    app.active_view = ActiveView::GitHubDiff;

    drop(app.update(Message::GitHubMergeFinished(
        repository.root.clone(),
        42,
        0,
        Ok("Merged pull request #42".into()),
    )));

    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert_eq!(panel.selected_pull_request_number, None);
    assert_eq!(
        panel.keyboard_focus,
        Some(GitHubPanelKeyboardFocus::PullRequestList)
    );
    assert_eq!(
        panel
            .pull_requests
            .as_ref()
            .expect("pull requests should be loaded")[0]
            .status,
        github::PullRequestSummaryStatus::Merged
    );
    assert!(panel.pull_requests_loading);
    assert!(app.github_diff.is_none());
    assert_eq!(app.active_view, ActiveView::Workspace);
}

#[test]
fn confirming_github_merge_locks_action_and_enters_loading_state() {
    let mut app = Muxtrix::new();
    let repository = github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    };
    let mut panel = GitHubPanelState::loading(repository);
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.loading = false;
    panel.merge_confirmation = true;
    panel.selected_pull_request_number = Some(42);
    panel.selected_pull_request = Some(github::PullRequestDetails {
        pull_request: github::PullRequest {
            number: 42,
            title: "Lock merge action".into(),
            url: "https://github.com/example/muxtrix/pull/42".into(),
            author: "octocat".into(),
            head: "merge-lock".into(),
            head_oid: "deadbeef".into(),
            head_repository: "example/muxtrix".into(),
            base: "main".into(),
            base_oid: "feedface".into(),
            additions: 1,
            deletions: 0,
            changed_files: 0,
            draft: false,
            mergeable: "MERGEABLE".into(),
            merge_state: "CLEAN".into(),
            review_decision: "APPROVED".into(),
            checks: github::CheckSummary {
                passed: 1,
                pending: 0,
                failed: 0,
            },
        },
        files: Vec::new(),
    });
    app.github_auth = github::AuthStatus::Authenticated {
        login: "octocat".into(),
    };
    app.github_panel = Some(panel);

    drop(app.update(Message::ConfirmGitHubMerge));

    let panel = app.github_panel.as_ref().expect("panel should remain open");
    assert!(panel.merging);
    assert!(!panel.merge_confirmation);
    assert!(panel.active_loading());
    assert!(panel.selected_pull_request_error.is_none());
}

#[test]
fn open_github_panel_does_not_steal_terminal_navigation_or_enter() {
    let mut app = Muxtrix::new();
    app.github_panel = Some(GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    }));

    assert!(
        app.handle_github_panel_keyboard(
            Key::Named(Named::ArrowDown).as_ref(),
            Modifiers::empty(),
        )
        .is_none()
    );
    assert!(
        app.handle_github_panel_keyboard(Key::Named(Named::Enter).as_ref(), Modifiers::empty(),)
            .is_none()
    );
}

#[test]
fn github_panel_tab_model_reaches_tabs_search_list_and_back() {
    let mut panel = GitHubPanelState::loading(github::Repository {
        root: std::env::temp_dir(),
        name: "muxtrix".into(),
        owner_and_name: Some("example/muxtrix".into()),
        host: "github.com".into(),
        branch: "main".into(),
        head_oid: String::new(),
        wsl_distribution: String::new(),
    });
    panel.active_tab = GitHubPanelTab::PullRequests;
    assert_eq!(
        github_keyboard_focus_step(&panel, GitHubPanelKeyboardFocus::Tabs, true),
        GitHubPanelKeyboardFocus::Search
    );
    assert_eq!(
        github_keyboard_focus_step(&panel, GitHubPanelKeyboardFocus::Search, true),
        GitHubPanelKeyboardFocus::PullRequestList
    );
    panel.selected_pull_request_number = Some(42);
    assert_eq!(
        github_keyboard_focus_step(&panel, GitHubPanelKeyboardFocus::Tabs, true),
        GitHubPanelKeyboardFocus::Back
    );
}

struct BlockingLauncher {
    entered: std::sync::mpsc::SyncSender<CreationDirectoryPolicy>,
    finished: std::sync::mpsc::SyncSender<()>,
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

type BlockingLauncherControl = (
    std::sync::mpsc::Receiver<CreationDirectoryPolicy>,
    std::sync::mpsc::Receiver<()>,
    Arc<(Mutex<bool>, std::sync::Condvar)>,
);
type TerminalCreationAction = fn(&mut Muxtrix) -> Result<(), String>;

impl TerminalLauncher for BlockingLauncher {
    fn launch(&self, request: TerminalLaunchRequest) -> Result<LaunchedTerminal, String> {
        let _ = self.entered.send(request.directory_policy);
        let (lock, ready) = &*self.gate;
        let mut released = lock.lock().expect("launch gate");
        while !*released {
            released = ready.wait(released).expect("launch gate wait");
        }
        let _ = self.finished.send(());
        Err("simulated terminal host stall".into())
    }
}

fn install_blocking_launcher(app: &mut Muxtrix) -> BlockingLauncherControl {
    let (entered_sender, entered_receiver) = std::sync::mpsc::sync_channel(1);
    let (finished_sender, finished_receiver) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    app.terminal_launcher = Arc::new(BlockingLauncher {
        entered: entered_sender,
        finished: finished_sender,
        gate: Arc::clone(&gate),
    });
    app.launch_in_background = true;
    (entered_receiver, finished_receiver, gate)
}

/// A PTY stand-in: it records every size the live session forwards, and
/// its reader parks instead of reporting the process as exited.
struct RecordingBackend {
    reader: Option<ParkedReader>,
    sizes: Arc<Mutex<Vec<PtySize>>>,
}

struct ParkedReader {
    bytes: std::sync::mpsc::Receiver<Vec<u8>>,
}

impl std::io::Read for ParkedReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let Ok(bytes) = self.bytes.recv() else {
            return Ok(0);
        };
        let count = bytes.len().min(out.len());
        out[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }
}

impl muxtrix_terminal::SessionBackend for RecordingBackend {
    fn take_reader(&mut self) -> Result<Box<dyn std::io::Read + Send>, String> {
        self.reader
            .take()
            .map(|reader| Box::new(reader) as Box<dyn std::io::Read + Send>)
            .ok_or_else(|| "the recording reader was already taken".to_owned())
    }
    fn write_all(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn resize(&self, size: PtySize) -> Result<(), String> {
        self.sizes.lock().expect("recorded sizes").push(size);
        Ok(())
    }
    fn kill(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    fn poll_exit(&mut self) -> Result<Option<bool>, String> {
        Ok(None)
    }
    fn exit_clean(&mut self) -> bool {
        true
    }
}

/// A daemon-like backend whose writer is stuck long enough to prove that
/// dropping UI state never joins its session actor on the calling thread.
struct SlowWriteBackend {
    reader: Option<ParkedReader>,
    entered: std::sync::mpsc::Sender<()>,
    finished: std::sync::mpsc::Sender<()>,
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl muxtrix_terminal::SessionBackend for SlowWriteBackend {
    fn take_reader(&mut self) -> Result<Box<dyn std::io::Read + Send>, String> {
        self.reader
            .take()
            .map(|reader| Box::new(reader) as Box<dyn std::io::Read + Send>)
            .ok_or_else(|| "the slow writer's reader was already taken".to_owned())
    }
    fn write_all(&mut self, _bytes: &[u8]) -> Result<(), String> {
        let _ = self.entered.send(());
        let (lock, ready) = &*self.gate;
        let mut released = lock.lock().expect("slow-write gate");
        while !*released {
            released = ready.wait(released).expect("slow-write gate wait");
        }
        Ok(())
    }
    fn resize(&self, _size: PtySize) -> Result<(), String> {
        Ok(())
    }
    fn kill(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    fn poll_exit(&mut self) -> Result<Option<bool>, String> {
        Ok(None)
    }
    fn exit_clean(&mut self) -> bool {
        false
    }
    fn kill_on_detach(&self) -> bool {
        false
    }
}

impl Drop for SlowWriteBackend {
    fn drop(&mut self) {
        let _ = self.finished.send(());
    }
}

#[test]
fn dropping_six_terminal_runtimes_never_waits_for_blocked_session_actors() {
    const PANE_COUNT: usize = 6;
    let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
    let (finished_sender, finished_receiver) = std::sync::mpsc::channel();
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let mut reader_lifetimes = Vec::new();
    let mut runtimes = Vec::new();

    for index in 0..PANE_COUNT {
        let (reader_sender, reader_receiver) = std::sync::mpsc::channel();
        reader_lifetimes.push(reader_sender);
        let session = LiveSession::spawn_remote(
            Box::new(SlowWriteBackend {
                reader: Some(ParkedReader {
                    bytes: reader_receiver,
                }),
                entered: entered_sender.clone(),
                finished: finished_sender.clone(),
                gate: Arc::clone(&gate),
            }),
            initial_pty_size(),
            TerminalOptions {
                cols: initial_pty_size().cols,
                rows: initial_pty_size().rows,
                max_scrollback: 10_000,
            },
            TerminalThemeId::default().preset().terminal_theme(),
            None,
        )
        .expect("the slow remote session should start");
        session
            .input(vec![b'0' + index as u8])
            .expect("the blocking write should be queued");
        let mut runtime = TerminalRuntime::suppressed("Shell");
        runtime.session = Some(session);
        runtime.launch_state = TerminalLaunchState::Running;
        runtimes.push(runtime);
    }

    for _ in 0..PANE_COUNT {
        entered_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("every session actor should enter its blocked write");
    }

    // The watchdog keeps a regressed implementation from hanging the test
    // forever. Correct teardown returns well before it opens the gate;
    // joining session actors on this thread can only return afterward.
    let release_gate = Arc::clone(&gate);
    let watchdog = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let (lock, ready) = &*release_gate;
        *lock.lock().expect("slow-write gate") = true;
        ready.notify_all();
    });
    let started = std::time::Instant::now();
    drop(runtimes);
    let drop_elapsed = started.elapsed();
    watchdog.join().expect("slow-write watchdog should finish");
    assert!(
        drop_elapsed < std::time::Duration::from_millis(500),
        "dropping GUI runtimes waited for their session actors"
    );

    // Let the detached PTY readers and disposal threads finish before the
    // test ends, without putting either wait back on the UI-drop path.
    drop(reader_lifetimes);
    for _ in 0..PANE_COUNT {
        finished_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("every detached session actor should finish");
    }
}

/// Launches a session over [`RecordingBackend`], mirroring how the system
/// launcher sizes a freshly spawned PTY.
#[derive(Clone, Default)]
struct RecordedLaunches {
    sizes: Arc<Mutex<Vec<PtySize>>>,
    scrollback_limits: Arc<Mutex<Vec<usize>>>,
}

struct RecordingLauncher {
    recorded: RecordedLaunches,
    idle: Mutex<Vec<std::sync::mpsc::Sender<Vec<u8>>>>,
}

impl TerminalLauncher for RecordingLauncher {
    fn launch(&self, request: TerminalLaunchRequest) -> Result<LaunchedTerminal, String> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.idle.lock().expect("idle readers").push(sender);
        self.recorded
            .scrollback_limits
            .lock()
            .expect("recorded scrollback limits")
            .push(request.max_scrollback);
        let backend = RecordingBackend {
            reader: Some(ParkedReader { bytes: receiver }),
            sizes: Arc::clone(&self.recorded.sizes),
        };
        let session = LiveSession::spawn_remote(
            Box::new(backend),
            initial_pty_size(),
            TerminalOptions {
                cols: initial_pty_size().cols,
                rows: initial_pty_size().rows,
                max_scrollback: request.max_scrollback,
            },
            request.theme,
            Some(request.notifier),
        )
        .map_err(|error| error.to_string())?;
        if terminal_grid_changed(initial_pty_size(), request.target_size) {
            session
                .resize(
                    request.target_size,
                    request.cell_width_px,
                    request.cell_height_px,
                )
                .map_err(|error| error.to_string())?;
        }
        let snapshot = session.snapshot().map_err(|error| error.to_string())?;
        Ok(LaunchedTerminal {
            session,
            snapshot,
            size: request.target_size,
            working_directory: request.profile.working_directory,
        })
    }
}

/// A daemon-owned pane stand-in: dropping one only detaches it, so a
/// recorded kill means the session was deliberately ended.
struct KillTrackingBackend {
    reader: Option<ParkedReader>,
    killed: Arc<AtomicBool>,
}

impl muxtrix_terminal::SessionBackend for KillTrackingBackend {
    fn take_reader(&mut self) -> Result<Box<dyn std::io::Read + Send>, String> {
        self.reader
            .take()
            .map(|reader| Box::new(reader) as Box<dyn std::io::Read + Send>)
            .ok_or_else(|| "the kill-tracking reader was already taken".to_owned())
    }
    fn write_all(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn resize(&self, _size: PtySize) -> Result<(), String> {
        Ok(())
    }
    fn kill(&mut self) -> Result<(), String> {
        self.killed.store(true, Ordering::Release);
        Ok(())
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    fn poll_exit(&mut self) -> Result<Option<bool>, String> {
        Ok(None)
    }
    fn exit_clean(&mut self) -> bool {
        false
    }
    fn kill_on_detach(&self) -> bool {
        false
    }
}

/// A launch result that arrives too late to be wanted. The returned
/// sender holds the session's reader open; dropping it would end the
/// session on its own and hide which disposal path ran.
fn late_launched_terminal(
    app: &Muxtrix,
    killed: &Arc<AtomicBool>,
) -> (LaunchedTerminal, std::sync::mpsc::Sender<Vec<u8>>) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let session = LiveSession::spawn_remote(
        Box::new(KillTrackingBackend {
            reader: Some(ParkedReader { bytes: receiver }),
            killed: Arc::clone(killed),
        }),
        initial_pty_size(),
        TerminalOptions {
            cols: initial_pty_size().cols,
            rows: initial_pty_size().rows,
            max_scrollback: 10_000,
        },
        app.settings.terminal_theme.preset().terminal_theme(),
        None,
    )
    .expect("the stand-in session should start");
    let snapshot = session.snapshot().expect("snapshot");
    (
        LaunchedTerminal {
            session,
            snapshot,
            size: initial_pty_size(),
            working_directory: None,
        },
        sender,
    )
}

fn wait_for_kill(killed: &Arc<AtomicBool>, within: std::time::Duration) -> bool {
    // Disposal runs off the UI thread.
    let deadline = std::time::Instant::now() + within;
    while std::time::Instant::now() < deadline {
        if killed.load(Ordering::Acquire) {
            return true;
        }
        std::thread::yield_now();
    }
    false
}

fn install_recording_launcher(app: &mut Muxtrix) -> RecordedLaunches {
    let recorded = RecordedLaunches::default();
    app.terminal_launcher = Arc::new(RecordingLauncher {
        recorded: recorded.clone(),
        idle: Mutex::new(Vec::new()),
    });
    app.launch_in_background = true;
    recorded
}

fn release_launcher(gate: &Arc<(Mutex<bool>, std::sync::Condvar)>) {
    let (lock, ready) = &**gate;
    *lock.lock().expect("launch gate") = true;
    ready.notify_all();
}

fn active_tab(app: &Muxtrix) -> &WorkspaceTab {
    app.active_workspace()
        .expect("workspace should exist")
        .active_tab()
        .expect("active tab should exist")
}

fn active_tab_mut(app: &mut Muxtrix) -> &mut WorkspaceTab {
    app.active_workspace_mut()
        .expect("workspace should exist")
        .active_tab_mut()
        .expect("active tab should exist")
}

fn active_pane_id(app: &Muxtrix) -> PaneId {
    active_tab(app).focused_pane_id
}

fn create_test_workspace(app: &mut Muxtrix) {
    app.workspace_name_draft = format!("Workspace {}", app.session.workspaces.len() + 1);
    app.create_workspace().expect("workspace should be created");
}

fn key_press(modified_key: Key, modifiers: Modifiers) -> KeyEvent {
    KeyEvent::Pressed(KeyInput {
        key: modified_key.clone(),
        modified_key,
        modifiers,
        text: None,
        repeat: false,
    })
}

#[test]
fn no_terminal_flag_is_explicit() {
    assert!(crate::no_terminal_requested(&[
        "muxtrix".into(),
        "--no-terminal".into()
    ]));
    assert!(!crate::no_terminal_requested(&["muxtrix".into()]));
}

#[test]
fn terminal_startup_surface_is_empty_and_uses_the_selected_theme() {
    let preparing = TerminalRuntime::preparing_host("shell");
    let starting = TerminalRuntime::starting("shell", 1, None);
    assert_eq!(terminal_empty_state_copy(Some(&preparing)), None);
    assert_eq!(terminal_empty_state_copy(Some(&starting)), None);

    let theme = TerminalThemeId::TokyoNight.preset();
    assert_eq!(terminal_surface_background(None, theme), theme.background);
}

#[test]
fn terminal_empty_state_keeps_actionable_failure_and_suppression_copy() {
    let suppressed = TerminalRuntime::suppressed("shell");
    assert_eq!(
        terminal_empty_state_copy(Some(&suppressed)),
        Some(suppressed.preview.as_str())
    );

    let mut failed = TerminalRuntime::starting("shell", 1, None);
    failed.preview = "Terminal unavailable".into();
    failed.launch_state = TerminalLaunchState::Failed("launch failed".into());
    assert_eq!(
        terminal_empty_state_copy(Some(&failed)),
        Some("Terminal unavailable")
    );
}

#[test]
fn production_does_not_use_an_implicit_local_pty_fallback() {
    assert!(!should_allow_local_pty(false, false, false));
    assert!(should_allow_local_pty(false, true, false));
    assert!(should_allow_local_pty(false, false, true));
    assert!(should_allow_local_pty(true, false, false));
}

#[test]
fn every_terminal_creation_handler_returns_while_the_launcher_is_hung() {
    fn split(app: &mut Muxtrix) -> Result<(), String> {
        app.split_terminal(SplitAxis::Horizontal)
    }
    fn new_tab(app: &mut Muxtrix) -> Result<(), String> {
        app.new_tab()
    }
    fn new_workspace(app: &mut Muxtrix) -> Result<(), String> {
        app.workspace_name_draft = "Recovery workspace".into();
        app.create_workspace()
    }
    fn restart(app: &mut Muxtrix) -> Result<(), String> {
        app.restart_pane(active_pane_id(app))
    }
    let actions: [(&str, TerminalCreationAction, CreationDirectoryPolicy); 4] = [
        ("split", split, CreationDirectoryPolicy::Regular),
        ("new tab", new_tab, CreationDirectoryPolicy::Regular),
        (
            "new workspace",
            new_workspace,
            CreationDirectoryPolicy::Regular,
        ),
        ("restart", restart, CreationDirectoryPolicy::Exact),
    ];

    for (name, action, expected_policy) in actions {
        let mut app = Muxtrix::new();
        let (entered, finished, gate) = install_blocking_launcher(&mut app);
        let previous_sidebar = app.sidebar_collapsed;
        let started = std::time::Instant::now();

        action(&mut app).unwrap_or_else(|error| panic!("{name} failed: {error}"));
        let pane_id = active_pane_id(&app);

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{name} waited for the terminal launcher"
        );
        let policy = entered
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap_or_else(|_| panic!("{name} launch did not enter the injected stall"));
        assert_eq!(
            policy, expected_policy,
            "{name} used the wrong working-directory policy"
        );
        assert!(matches!(
            app.terminals[&pane_id].launch_state,
            TerminalLaunchState::Starting { .. }
        ));
        let _ = app.update(Message::ToggleSidebar);
        assert_ne!(app.sidebar_collapsed, previous_sidebar);

        release_launcher(&gate);
        finished
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap_or_else(|_| panic!("{name} launch did not return after release"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            app.poll_terminal();
            if matches!(
                app.terminals[&pane_id].launch_state,
                TerminalLaunchState::Failed(_)
            ) {
                break;
            }
            std::thread::yield_now();
        }
        assert!(matches!(
            app.terminals[&pane_id].launch_state,
            TerminalLaunchState::Failed(ref error)
                if error == "simulated terminal host stall"
        ));
    }
}

#[test]
fn restarting_a_fresh_tab_waits_for_its_in_flight_launch() {
    let mut app = Muxtrix::new();
    let (entered, finished, gate) = install_blocking_launcher(&mut app);
    app.new_tab().expect("fresh tab should be created");
    let pane_id = active_pane_id(&app);
    let first_attempt = app.next_terminal_launch_attempt;
    let first_policy = entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("the fresh tab launch should enter the worker");
    assert_eq!(first_policy, CreationDirectoryPolicy::Regular);

    let target = std::env::temp_dir().join("muxtrix-queued-restart-target");
    app.restart_pane_in_directory(pane_id, target.clone())
        .expect("restart should queue behind the fresh tab launch");

    assert_eq!(
        app.next_terminal_launch_attempt, first_attempt,
        "the replacement must not overlap a launch using the same pane ID"
    );
    assert!(app.queued_terminal_restarts.contains(&pane_id));
    let terminal = app
        .active_workspace()
        .expect("workspace")
        .pane(pane_id)
        .and_then(Pane::active_surface)
        .and_then(|surface| match &surface.kind {
            muxtrix_domain::SurfaceKind::Terminal(terminal) => Some(terminal),
            _ => None,
        })
        .expect("terminal surface");
    assert_eq!(terminal.working_directory.as_ref(), Some(&target));

    release_launcher(&gate);
    finished
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("the original launch should finish after release");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while app.next_terminal_launch_attempt == first_attempt && std::time::Instant::now() < deadline
    {
        app.drain_terminal_launches();
        std::thread::yield_now();
    }

    assert_eq!(app.next_terminal_launch_attempt, first_attempt + 1);
    assert!(!app.queued_terminal_restarts.contains(&pane_id));
    let replacement_policy = entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("the queued replacement should enter the worker");
    assert_eq!(replacement_policy, CreationDirectoryPolicy::Exact);
    finished
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("the queued replacement should launch after the original completes");
    app.drain_terminal_launches();
}

#[test]
fn cancelling_a_hung_launch_ignores_its_late_completion() {
    let mut app = Muxtrix::new();
    let (entered, finished, gate) = install_blocking_launcher(&mut app);
    app.split_terminal(SplitAxis::Vertical)
        .expect("pane creation should enqueue a launch");
    let pane_id = active_pane_id(&app);
    let _ = entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("launch worker should enter the injected stall");

    let _ = app.update(Message::CancelTerminalLaunch(pane_id));
    release_launcher(&gate);
    finished
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("launch worker should return after release");
    let completion_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < completion_deadline
        && app
            .terminal_launch_completions
            .lock()
            .is_ok_and(|queue| queue.is_empty())
    {
        std::thread::yield_now();
    }
    app.poll_terminal();
    assert!(
        app.terminal_launch_completions
            .lock()
            .expect("completion queue")
            .is_empty()
    );
    assert!(matches!(
        app.terminals[&pane_id].launch_state,
        TerminalLaunchState::Suppressed
    ));
}

#[test]
fn a_launch_landing_after_its_pane_closed_ends_the_session_it_produced() {
    let mut app = Muxtrix::new();
    let killed = Arc::new(AtomicBool::new(false));
    let (launched, reader) = late_launched_terminal(&app, &killed);
    let pane_id = active_pane_id(&app);
    // The close happened while this launch was still in flight, so it
    // found no session to end.
    app.terminals.remove(&pane_id);

    app.finish_terminal_launch(TerminalLaunchCompletion {
        pane_id,
        attempt_id: app.next_terminal_launch_attempt.wrapping_add(1),
        result: Ok(launched),
    });

    assert!(
        wait_for_kill(&killed, std::time::Duration::from_secs(5)),
        "a session no pane can reach must be killed, not merely detached"
    );
    drop(reader);
}

#[test]
fn a_superseded_launch_detaches_without_killing_the_pane_that_replaced_it() {
    let mut app = Muxtrix::new();
    let killed = Arc::new(AtomicBool::new(false));
    let (launched, reader) = late_launched_terminal(&app, &killed);
    let pane_id = active_pane_id(&app);
    assert!(
        app.terminals.contains_key(&pane_id),
        "the pane must still be open for this test to mean anything"
    );

    app.finish_terminal_launch(TerminalLaunchCompletion {
        pane_id,
        attempt_id: app.next_terminal_launch_attempt.wrapping_add(1),
        result: Ok(launched),
    });

    // A newer attempt shares this pane's identity with the daemon, so a
    // kill here would take down the session that replaced this one.
    assert!(
        !wait_for_kill(&killed, std::time::Duration::from_millis(500)),
        "a superseded launch must detach, not kill the pane it lost to"
    );
    drop(reader);
}

#[test]
fn a_pane_measured_during_its_launch_keeps_that_size_once_the_session_arrives() {
    let mut app = Muxtrix::new();
    app.settings.terminal_scrollback_lines = 25_000;
    let recorded = install_recording_launcher(&mut app);
    app.split_terminal(SplitAxis::Horizontal)
        .expect("pane creation should enqueue a launch");
    let pane_id = active_pane_id(&app);

    // Layout measures the new pane while its launch is still in flight,
    // so there is no session yet to forward the size to.
    let pane_size = Size::new(420.0, 260.0);
    let _ = app.update(Message::ResizePane(pane_id, pane_size));
    let expected = pty_size_for_pane(pane_size, &app.settings);
    assert!(
        terminal_grid_changed(initial_pty_size(), expected),
        "the pane must differ from the launch default for this test to mean anything"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && app
            .terminal_launch_completions
            .lock()
            .is_ok_and(|queue| queue.is_empty())
    {
        std::thread::yield_now();
    }
    app.poll_terminal();

    let runtime = &app.terminals[&pane_id];
    assert!(runtime.session.is_some(), "the launch should have landed");
    assert_eq!(
        runtime.size, expected,
        "the launch must not restore the size it was requested with"
    );

    // The live session forwards resizes on its own thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && recorded
            .sizes
            .lock()
            .expect("recorded sizes")
            .last()
            .is_none_or(|size| terminal_grid_changed(*size, expected))
    {
        std::thread::yield_now();
    }
    assert_eq!(
        recorded
            .sizes
            .lock()
            .expect("recorded sizes")
            .last()
            .copied(),
        Some(expected),
        "the PTY should end up sized to the pane it is drawn into"
    );
    assert_eq!(
        *recorded
            .scrollback_limits
            .lock()
            .expect("recorded scrollback limits"),
        vec![25_000],
        "new panes should use the configured scrollback history"
    );
}

#[test]
fn update_splits_without_a_window() {
    let mut app = Muxtrix::new();
    let original_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let workspace = app.active_workspace().expect("workspace should exist");
    let tab = workspace.active_tab().expect("active tab should exist");
    assert_eq!(tab.panes.len(), 2);
    assert_ne!(tab.focused_pane_id, original_pane);
    assert_eq!(app.terminals.len(), 2);
    assert!(
        app.terminals
            .values()
            .all(|runtime| runtime.session.is_some())
    );
    workspace.validate().expect("workspace should stay valid");
}

#[test]
fn closing_a_pane_drops_only_its_terminal_runtime() {
    let mut app = Muxtrix::new();
    let original_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Vertical));
    let closed_pane = active_pane_id(&app);

    let _ = app.update(Message::ClosePane(closed_pane));

    assert_eq!(app.terminals.len(), 1);
    assert!(app.terminals.contains_key(&original_pane));
    assert!(!app.terminals.contains_key(&closed_pane));
}

/// A pane can be closed while the pointer still rests over it, so the
/// hover must die with the pane — a stale one aims the next wheel event
/// at a pane that no longer exists.
#[test]
fn closing_the_hovered_pane_clears_the_hover() {
    let mut app = Muxtrix::new();
    let _ = app.update(Message::Split(SplitAxis::Vertical));
    let closed_pane = active_pane_id(&app);
    app.hovered_terminal = Some(closed_pane);

    let _ = app.update(Message::ClosePane(closed_pane));
    assert_eq!(app.hovered_terminal, None);

    assert!(
        !app.status.contains("Terminal scroll failed"),
        "scrolling after the hovered pane closed must not report a missing pane: {}",
        app.status
    );
}

#[test]
fn shell_exit_detaches_the_dead_session_and_restart_replaces_it() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);

    // A live PTY is ready before its shell has necessarily initialized its
    // line editor. Waiting for the first non-empty frame makes the test
    // exercise a user-issued `exit` instead of racing process startup when
    // the Windows suite launches several ConPTY sessions concurrently,
    // and loaded CI runners have been seen to blow well past 10s.
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty()),
        "shell did not render a prompt before the readiness deadline"
    );
    app.send_terminal_input(b"exit\r".to_vec())
        .expect("shell should accept exit");

    let exit_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < exit_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id].session.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        app.terminals[&pane_id].session.is_none(),
        "shell remained attached ten seconds after receiving exit"
    );
    app.terminals
        .get_mut(&pane_id)
        .expect("runtime should remain visible")
        .resize(Size::new(800.0, 500.0), &app.settings)
        .expect("an exited pane should still resize without targeting a dead channel");

    app.restart_pane(pane_id)
        .expect("the exited terminal should restart");
    assert!(app.terminals[&pane_id].session.is_some());
}

#[cfg(unix)]
#[test]
fn clean_terminal_exit_closes_its_pane_and_cascades() {
    let mut app = Muxtrix::new();
    let original = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let split = active_pane_id(&app);
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&split]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.send_terminal_input(b"exit\r".to_vec())
        .expect("shell should accept exit");

    let close_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < close_deadline {
        app.poll_terminal();
        if !app.terminals.contains_key(&split) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !app.terminals.contains_key(&split),
        "a cleanly exited pane should close itself"
    );
    assert!(app.terminals.contains_key(&original));
    // With other panes present, no workspace confirmation appears.
    assert!(app.close_workspace_prompt.is_none());
    let workspace = app.active_workspace().expect("workspace should remain");
    assert_eq!(workspace.pane_count(), 1);
    workspace.validate().expect("workspace should stay valid");
}

#[cfg(unix)]
#[test]
fn focused_split_receives_output_independently() {
    let mut app = Muxtrix::new();
    let original_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let focused_pane = active_pane_id(&app);

    app.send_terminal_input(b"printf 'focused-pane-marker\\n'\r".to_vec())
        .expect("focused terminal should accept input");

    let mut focused_text = String::new();
    for _ in 0..200 {
        app.poll_terminal();
        focused_text = app
            .terminals
            .get(&focused_pane)
            .and_then(|runtime| runtime.snapshot.as_ref())
            .map(GridSnapshot::text)
            .unwrap_or_default();
        if focused_text.contains("focused-pane-marker") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let original_text = app
        .terminals
        .get(&original_pane)
        .and_then(|runtime| runtime.snapshot.as_ref())
        .map(GridSnapshot::text)
        .unwrap_or_default();
    assert!(focused_text.contains("focused-pane-marker"));
    assert!(!original_text.contains("focused-pane-marker"));
}

#[cfg(unix)]
#[test]
fn osc_title_updates_the_pane_and_native_window_title() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let profile = app
        .session
        .profiles
        .first_mut()
        .expect("terminal profile should exist");
    profile.program = "/bin/sh".into();
    profile.arguments = vec![
        "-c".into(),
        "printf '\\033]2;cargo test\\007'; sleep 5".into(),
    ];
    app.restart_pane(pane_id)
        .expect("title-emitting terminal should start");

    for _ in 0..200 {
        app.poll_terminal();
        if app.pane_title(
            app.active_workspace().expect("workspace should exist"),
            pane_id,
        ) == "cargo test"
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let workspace = app.active_workspace().expect("workspace should exist");
    assert_eq!(app.pane_title(workspace, pane_id), "cargo test");
    assert_eq!(app.title(), "cargo test — Tab 1 — Muxtrix");
}

#[test]
fn terminal_style_runs_coalesce_uniform_cells_before_gpu_projection() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal actor should start");
    actor
        .feed(b"plain \x1b[31mred\x1b[0m text".to_vec())
        .expect("terminal should accept styled text");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let cell_fragments = snapshot.cells.iter().map(|row| row.len()).sum::<usize>()
        + snapshot.cells.len().saturating_sub(1);
    let runs = terminal_style_runs(&snapshot, false, true, TerminalThemeId::Ghostty.preset());

    assert!(
        runs.len() * 20 < cell_fragments,
        "expected style runs to replace most cell spans: {} runs for {cell_fragments} cells",
        runs.len()
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn unicode_activity_frames_keep_following_text_on_fixed_grid_columns() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 24,
        rows: 2,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed("⠋ Working".as_bytes().to_vec())
        .expect("terminal should accept a unicode activity frame");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let rows = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        None,
        TerminalThemeId::Ghostty.preset(),
    );
    let first_row = &rows[0];

    assert!(first_row.iter().any(|run| {
        run.text == "⠋" && run.columns == 1 && run.kind == TerminalRunKind::IsolatedUnicode
    }));
    assert!(first_row.iter().any(|run| run.text.starts_with(" Working")));
    assert_eq!(
        first_row.iter().map(|run| run.columns).sum::<usize>(),
        snapshot.cells[0].len(),
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn repeated_box_and_block_glyphs_use_continuous_geometry_runs() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 32,
        rows: 2,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed("──────── █████░░░".as_bytes().to_vec())
        .expect("terminal should accept terminal drawing glyphs");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let rows = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        None,
        TerminalThemeId::Ghostty.preset(),
    );
    let first_row = &rows[0];

    assert!(first_row.iter().any(|run| {
        run.text == "────────" && run.columns == 8 && run.kind == TerminalRunKind::BoxDrawing
    }));
    assert!(first_row.iter().any(|run| {
        run.text == "█████" && run.columns == 5 && run.kind == TerminalRunKind::JoinedCellGlyph('█')
    }));
    assert!(first_row.iter().any(|run| {
        run.text == "░░░" && run.columns == 3 && run.kind == TerminalRunKind::JoinedCellGlyph('░')
    }));
    assert_eq!(
        first_row.iter().map(|run| run.columns).sum::<usize>(),
        snapshot.cells[0].len(),
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn rounded_box_borders_group_corners_and_horizontal_arms_together() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 16,
        rows: 3,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed("╭──────╮\r\n│ Codex│\r\n╰──────╯".as_bytes().to_vec())
        .expect("terminal should accept a rounded box");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let rows = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        None,
        TerminalThemeId::Ghostty.preset(),
    );

    for (row, expected) in [(0, "╭──────╮"), (2, "╰──────╯")] {
        let border = rows[row]
            .iter()
            .find(|run| run.text == expected)
            .expect("rounded border should be one geometry run");
        assert_eq!(border.columns, 8);
        assert_eq!(border.kind, TerminalRunKind::BoxDrawing);
    }
    assert!(rows[1].iter().any(|run| {
        run.text == "│" && run.columns == 1 && run.kind == TerminalRunKind::BoxDrawing
    }));
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn wide_unicode_cells_own_their_spacer_column_in_fixed_grid_projection() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 12,
        rows: 2,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed("界 ok".as_bytes().to_vec())
        .expect("terminal should accept a wide glyph");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let rows = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        None,
        TerminalThemeId::Ghostty.preset(),
    );

    assert!(
        rows[0]
            .iter()
            .any(|run| run.text == "界" && run.columns == 2)
    );
    assert_eq!(
        rows[0].iter().map(|run| run.columns).sum::<usize>(),
        snapshot.cells[0].len(),
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn terminal_selection_maps_pointer_cells_and_marks_only_the_selected_range() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 8,
        rows: 2,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed(b"abcdef".to_vec())
        .expect("terminal should accept text");
    actor
        .selection_start(1, 0)
        .expect("selection should anchor");
    actor
        .selection_extend(3, 0)
        .expect("selection should extend");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let runs = terminal_style_runs(&snapshot, false, true, TerminalThemeId::Ghostty.preset());
    assert!(
        runs.iter()
            .any(|run| run.style.selected && run.text == "bcd")
    );
    assert_eq!(
        actor.selection_text().expect("selection text").as_deref(),
        Some("bcd"),
        "copy should come from the emulator that owns the selection"
    );

    let settings = AppSettings::default();
    assert_eq!(
        terminal_cell_at(
            Point::new(
                8.0 + settings.terminal_cell_width() * 2.2,
                8.0 + settings.terminal_cell_height() * 1.2,
            ),
            &settings,
            0,
        ),
        TerminalCellPosition { row: 1, column: 2 }
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn a_pointer_maps_to_the_grid_cell_it_is_over() {
    // Selection speaks to the emulator in viewport cells, so this mapping
    // is the whole of what this side contributes to it.
    let settings = AppSettings::default();
    let size = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 800,
        pixel_height: 480,
    };
    assert_eq!(
        terminal_grid_cell_at(
            Point::new(
                TERMINAL_PADDING / 2.0 + settings.terminal_cell_width() * 2.2,
                TERMINAL_PADDING / 2.0 + settings.terminal_cell_height() * 1.2,
            ),
            &settings,
            size,
        ),
        (2, 1)
    );
    assert_eq!(
        terminal_grid_cell_at(Point::new(-40.0, -40.0), &settings, size),
        (0, 0),
        "a pointer above and left of the grid still lands on its first cell"
    );
    assert_eq!(
        terminal_grid_cell_at(Point::new(100_000.0, 100_000.0), &settings, size),
        (size.cols - 1, size.rows - 1),
        "and one beyond it lands on the last, never off the grid"
    );
}

#[test]
fn terminal_click_clears_selection_while_a_real_drag_starts_one() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let origin = Point::new(120.0, 80.0);
    app.terminals
        .get_mut(&pane_id)
        .expect("the initial pane should have a runtime")
        .has_selection = true;

    let _ = app.update(Message::TerminalPointerMoved(pane_id, origin));
    let _ = app.update(Message::TerminalMousePressed(
        pane_id,
        TerminalMouseButton::Left,
    ));
    assert!(
        !app.terminals[&pane_id].has_selection,
        "mouse-down should dismiss the previous selection immediately"
    );
    assert!(
        app.terminal_selection_drag
            .is_some_and(|drag| drag.pane_id == pane_id && !drag.active)
    );

    let click_jitter = Point::new(origin.x + 1.0, origin.y + 1.0);
    let _ = app.update(Message::TerminalPointerMoved(pane_id, click_jitter));
    let _ = app.update(Message::TerminalMouseReleased(
        pane_id,
        TerminalMouseButton::Left,
    ));
    assert!(app.terminal_selection_drag.is_none());
    assert!(
        !app.terminals[&pane_id].has_selection,
        "sub-threshold click jitter must not create a one-cell selection"
    );

    let _ = app.update(Message::TerminalPointerMoved(pane_id, origin));
    let _ = app.update(Message::TerminalMousePressed(
        pane_id,
        TerminalMouseButton::Left,
    ));
    let _ = app.update(Message::TerminalPointerMoved(
        pane_id,
        Point::new(origin.x + TERMINAL_SELECTION_DRAG_THRESHOLD + 1.0, origin.y),
    ));
    assert!(
        app.terminal_selection_drag
            .is_some_and(|drag| drag.pane_id == pane_id && drag.active)
    );
    assert!(
        app.terminals[&pane_id].has_selection,
        "crossing the drag threshold should still select terminal text"
    );
    assert_eq!(
        app.finish_terminal_selection(None),
        Some(pane_id),
        "a genuine drag should commit when the window observes its release"
    );
    assert!(app.terminal_selection_drag.is_none());
    assert_eq!(
        app.finish_terminal_selection(Some(pane_id)),
        None,
        "the pane-level release must not schedule a duplicate copy"
    );
}

#[test]
fn mouse_reporting_reserves_plain_clicks_and_shift_restores_selection() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.terminals
        .get_mut(&pane_id)
        .expect("the initial pane should have a runtime")
        .snapshot = Some(snapshot_in_mode(b"\x1b[?1003h\x1b[?1006h"));
    let position = Point::new(120.0, 80.0);
    let _ = app.update(Message::TerminalPointerMoved(pane_id, position));

    let _ = app.update(Message::TerminalMousePressed(
        pane_id,
        TerminalMouseButton::Left,
    ));
    assert!(
        app.terminal_selection_drag.is_none(),
        "an unmodified click belongs to the mouse-reporting program"
    );

    app.keyboard_modifiers = Modifiers::SHIFT;
    let _ = app.update(Message::TerminalMousePressed(
        pane_id,
        TerminalMouseButton::Left,
    ));
    assert!(
        app.terminal_selection_drag
            .is_some_and(|drag| drag.pane_id == pane_id),
        "Shift must escape program mouse capture for local text selection"
    );
}

/// A snapshot rendered by a terminal in the given screen mode.
fn snapshot_in_mode(sequence: &[u8]) -> GridSnapshot {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 24,
        rows: 4,
        max_scrollback: 100,
    })
    .expect("terminal actor should start");
    let mut bytes = sequence.to_vec();
    bytes.extend_from_slice(b"selected text");
    actor.feed(bytes).expect("terminal should accept bytes");
    let snapshot = actor.snapshot().expect("snapshot should render");
    actor.shutdown().expect("terminal actor should stop");
    snapshot
}

#[test]
fn cursor_and_selection_backgrounds_use_the_overlay_plane() {
    let mut snapshot = snapshot_in_mode(b"");
    let theme = TerminalThemeId::Ghostty.preset();
    let cursor = snapshot.cursor.expect("visible cursor position");
    let cursor_color = snapshot.cursor_color.unwrap_or(theme.cursor);
    let runs = terminal_row_style_runs(&snapshot, true, true, None, theme);
    let cursor_run = runs[usize::from(cursor.row)]
        .iter()
        .find(|run| run.style.overlay_background == Some(cursor_color))
        .expect("cursor overlay");
    assert_eq!(cursor_run.style.background, None);

    snapshot.selection[usize::from(cursor.row)] = Some(muxtrix_terminal::SelectedColumns {
        start: usize::from(cursor.column),
        end: usize::from(cursor.column),
    });
    let selected_runs = terminal_row_style_runs(&snapshot, true, true, None, theme);
    let selected_cursor = selected_runs[usize::from(cursor.row)]
        .iter()
        .find(|run| run.style.selected)
        .expect("selected cursor cell");
    assert_eq!(
        selected_cursor.style.overlay_background,
        Some(theme.selection_background)
    );
    assert_eq!(selected_cursor.style.background, None);
}

#[test]
fn scrolling_never_clears_selection_at_the_application_layer() {
    // Selection movement belongs to the terminal session for every screen
    // mode. The app must not discard it before the emulator sees the
    // application's response to the wheel.
    for (sequence, application_scroll) in [
        (b"\x1b[?1049h".as_slice(), true),
        (b"\x1b[?1000h".as_slice(), true),
        (b"".as_slice(), false),
    ] {
        let mut app = Muxtrix::new();
        let pane_id = active_pane_id(&app);
        let snapshot = snapshot_in_mode(sequence);
        assert_eq!(snapshot.application_scroll, application_scroll);
        let runtime = app
            .terminals
            .get_mut(&pane_id)
            .expect("the initial pane should have a runtime");
        runtime.snapshot = Some(snapshot);
        runtime.has_selection = true;

        // The wheel itself fails without a live session; whether the app
        // discarded the selection before trying it is what matters here.
        let _ = app.scroll_terminal(pane_id, ScrollDelta::Lines { x: 0.0, y: -3.0 });

        assert!(
            app.terminals[&pane_id].has_selection,
            "screen mode {application_scroll} must not make the app clear \
             emulator-owned selection state"
        );
    }
}

#[test]
fn web_urls_are_dotted_by_default_and_solid_when_clickable() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 64,
        rows: 2,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed(b"See (https://example.com/docs?q=1).".to_vec())
        .expect("terminal should accept a URL");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let link = terminal_link_at(&snapshot, TerminalCellPosition { row: 0, column: 12 })
        .expect("URL should be detected under the pointer");

    assert_eq!(link.uri, "https://example.com/docs?q=1");
    assert_eq!(link.start_column, 5);
    assert_eq!(link.end_column, 33);
    assert!(terminal_link_modifiers(Modifiers::CTRL | Modifiers::SHIFT));
    assert!(!terminal_link_modifiers(Modifiers::CTRL));
    assert!(!terminal_link_modifiers(
        Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALT
    ));

    let default_runs = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        None,
        TerminalThemeId::Ghostty.preset(),
    );
    assert_eq!(
        default_runs[0]
            .iter()
            .filter(|run| run.style.link && !run.style.link_hovered)
            .map(|run| run.columns)
            .sum::<usize>(),
        link.end_column - link.start_column
    );

    let runs = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        Some(&link),
        TerminalThemeId::Ghostty.preset(),
    );
    assert_eq!(
        runs[0]
            .iter()
            .filter(|run| run.style.link && run.style.link_hovered)
            .map(|run| run.columns)
            .sum::<usize>(),
        link.end_column - link.start_column
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn inferred_urls_override_application_underlines_until_clickable() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 64,
        rows: 2,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed(b"\x1b[4mnote https://example.com\x1b[24m".to_vec())
        .expect("terminal should accept an underlined URL");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let link = terminal_link_at(&snapshot, TerminalCellPosition { row: 0, column: 10 })
        .expect("underlined URL should be inferred under the pointer");

    let default_runs = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        None,
        TerminalThemeId::Ghostty.preset(),
    );
    let underlined_text = default_runs[0]
        .iter()
        .find(|run| run.text.contains("note"))
        .expect("ordinary underlined text should remain present");
    let inferred_url = default_runs[0]
        .iter()
        .find(|run| run.style.link)
        .expect("the inferred URL should have a link run");
    assert!(inferred_url.style.underline);
    assert_eq!(
        terminal_underline_decoration(underlined_text.style),
        TerminalUnderlineDecoration::Solid
    );
    assert_eq!(
        terminal_underline_decoration(inferred_url.style),
        TerminalUnderlineDecoration::Dotted
    );

    let hovered_runs = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        Some(&link),
        TerminalThemeId::Ghostty.preset(),
    );
    assert!(
        hovered_runs[0]
            .iter()
            .filter(|run| run.style.link)
            .all(|run| terminal_underline_decoration(run.style)
                == TerminalUnderlineDecoration::Solid)
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn osc8_link_uses_its_destination_instead_of_visible_label() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 32,
        rows: 2,
        max_scrollback: 10,
    })
    .expect("terminal actor should start");
    actor
        .feed(b"\x1b]8;;https://example.com/target\x1b\\open docs\x1b]8;;\x1b\\".to_vec())
        .expect("terminal should accept an OSC 8 link");
    let snapshot = actor.snapshot().expect("snapshot should render");
    let link = terminal_link_at(&snapshot, TerminalCellPosition { row: 0, column: 4 })
        .expect("OSC 8 link should be detected under its label");

    assert_eq!(link.uri, "https://example.com/target");
    assert_eq!((link.start_column, link.end_column), (0, 9));
    let runs = terminal_row_style_runs(
        &snapshot,
        false,
        true,
        None,
        TerminalThemeId::Ghostty.preset(),
    );
    assert_eq!(
        runs[0]
            .iter()
            .filter(|run| run.style.link && !run.style.link_hovered)
            .map(|run| run.columns)
            .sum::<usize>(),
        9
    );
    actor.shutdown().expect("terminal actor should stop");
}

#[test]
fn terminal_scrollbar_geometry_maps_track_extremes_to_scrollback() {
    let scrollbar = ScrollbarSnapshot {
        total: 240,
        visible: 40,
        offset: 100,
    };
    let geometry = terminal_scrollbar_geometry(scrollbar, 410.0);

    assert_eq!(geometry.max_offset, 200);
    assert!(geometry.thumb_height >= 24.0);
    assert_eq!(geometry.offset_for_thumb_top(-100.0), 0);
    assert_eq!(
        geometry.offset_for_thumb_top(geometry.track_height),
        geometry.max_offset
    );
}

#[test]
fn palette_selection_wraps_and_handles_empty_results() {
    assert_eq!(palette_selection(0, 7, PaletteMove::Next), 1);
    assert_eq!(palette_selection(6, 7, PaletteMove::Next), 0);
    assert_eq!(palette_selection(0, 7, PaletteMove::Previous), 6);
    assert_eq!(palette_selection(4, 0, PaletteMove::Previous), 0);
}

#[test]
fn palette_selection_skips_disabled_commands() {
    let enabled = [false, false, true, false, true];
    assert_eq!(first_enabled_palette_command(&enabled), 2);
    assert_eq!(enabled_palette_selection(2, &enabled, PaletteMove::Next), 4);
    assert_eq!(enabled_palette_selection(4, &enabled, PaletteMove::Next), 2);
    assert_eq!(
        enabled_palette_selection(2, &enabled, PaletteMove::Previous),
        4
    );
    assert_eq!(enabled_palette_selection(0, &enabled, PaletteMove::Next), 2);
    assert_eq!(
        enabled_palette_selection(0, &enabled, PaletteMove::Previous),
        4
    );
    assert_eq!(first_enabled_palette_command(&[false, false]), 0);
    assert_eq!(
        enabled_palette_selection(1, &[false, false], PaletteMove::Next),
        0
    );
}

#[test]
fn terminal_keys_encode_text_navigation_and_control() {
    assert_eq!(
        encode_terminal_key(Key::Character("é"), Modifiers::empty(), Some("é")),
        Some("é".as_bytes().to_vec())
    );
    assert_eq!(
        encode_terminal_key(Key::Character("c"), Modifiers::CTRL, None),
        Some(vec![0x03])
    );
    assert_eq!(
        encode_terminal_key(Key::Named(Named::Enter), Modifiers::empty(), None),
        Some(vec![b'\r'])
    );
    assert_eq!(
        encode_terminal_key(Key::Named(Named::Space), Modifiers::empty(), None),
        Some(vec![b' '])
    );
    assert_eq!(
        encode_terminal_key(Key::Named(Named::Space), Modifiers::CTRL, None),
        Some(vec![0x00])
    );
    assert_eq!(
        encode_terminal_key(Key::Named(Named::ArrowLeft), Modifiers::CTRL, None),
        Some(b"\x1b[1;5D".to_vec())
    );
}

#[test]
fn terminal_keys_support_meta_and_ignore_window_shortcuts() {
    assert_eq!(
        encode_terminal_key(Key::Character("x"), Modifiers::ALT, Some("x")),
        Some(b"\x1bx".to_vec())
    );
    assert_eq!(
        encode_terminal_key(Key::Character("x"), Modifiers::LOGO, Some("x")),
        None
    );
}

#[test]
fn rename_prompt_keystrokes_never_reach_the_terminal() {
    let mut app = Muxtrix::new();
    app.rename_prompt = Some(RenameTarget::Workspace(app.session.active_workspace_id));
    let _ = app.handle_keyboard(KeyEvent::Pressed(KeyInput {
        key: Key::Character("x".into()),
        modified_key: Key::Character("x".into()),
        modifiers: Modifiers::empty(),
        text: Some("x".into()),
        repeat: false,
    }));
    assert!(app.terminal_command_buffers.is_empty());
}

#[test]
fn palette_rename_updates_workspaces_tabs_and_panes() {
    let mut app = Muxtrix::new();
    let workspace_id = app.session.active_workspace_id;
    let _ = app.run_command(CommandAction::RenameWorkspace);
    assert_eq!(
        app.rename_prompt,
        Some(RenameTarget::Workspace(workspace_id))
    );
    app.rename_draft = "gateway".into();
    app.apply_rename().expect("workspace rename should apply");
    assert_eq!(app.active_workspace().expect("workspace").name, "gateway");

    let tab_id = app.active_workspace().expect("workspace").active_tab_id;
    let _ = app.run_command(CommandAction::RenameTab);
    assert_eq!(
        app.rename_prompt,
        Some(RenameTarget::Tab(workspace_id, tab_id))
    );
    app.rename_draft = "review".into();
    app.apply_rename().expect("tab rename should apply");
    assert_eq!(
        app.active_workspace()
            .expect("workspace")
            .active_tab()
            .expect("tab")
            .name,
        "review"
    );

    let pane_id = active_pane_id(&app);
    let _ = app.run_command(CommandAction::RenamePane);
    assert_eq!(app.rename_prompt, Some(RenameTarget::Pane(pane_id)));
    app.rename_draft = "build watcher".into();
    app.apply_rename().expect("pane rename should apply");
    let workspace = app.active_workspace().expect("workspace");
    assert_eq!(app.pane_title(workspace, pane_id), "build watcher");

    // An empty pane name clears the override back to the automatic title.
    app.rename_prompt = Some(RenameTarget::Pane(pane_id));
    app.rename_draft = "  ".into();
    app.apply_rename().expect("pane rename should clear");
    let workspace = app.active_workspace().expect("workspace");
    assert_eq!(app.pane_title(workspace, pane_id), "shell 1");
}

#[test]
fn agent_titles_prefer_harness_or_worktree_identity_over_the_brand() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "codex".into(),
            display_name: Some("agent-title-hardening".into()),
            state: AgentState::Running,
            activity: None,
            session_id: None,
            cwd: None,
            git_branch: None,
        },
    );
    let workspace = app.active_workspace().expect("workspace");
    assert_eq!(app.pane_title(workspace, pane_id), "agent-title-hardening");

    app.agent_statuses
        .get_mut(&pane_id)
        .expect("agent status")
        .display_name = None;
    let workspace = app.active_workspace().expect("workspace");
    assert_eq!(app.pane_title(workspace, pane_id), "Codex");
}

#[test]
fn session_layout_persists_agent_identity_for_reattach() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: Some("resume-status".into()),
            state: AgentState::Running,
            activity: None,
            session_id: Some("session-1".into()),
            cwd: None,
            git_branch: None,
        },
    );

    let persisted = session_with_agent_identities(&app.session, &app.agent_statuses);
    assert_eq!(
        persisted.workspaces[0].tabs[0].panes[&pane_id].agent,
        Some(PaneAgent::ClaudeCode)
    );
    let restored = agent_statuses_from_session(&persisted);
    assert_eq!(restored[&pane_id].agent, "claude");
    assert_eq!(restored[&pane_id].state, AgentState::Idle);

    // Removing the live status must clear a previously persisted identity
    // on the next layout sync instead of preserving a stale agent forever.
    let cleared = session_with_agent_identities(&persisted, &BTreeMap::new());
    assert_eq!(cleared.workspaces[0].tabs[0].panes[&pane_id].agent, None);
}

#[test]
fn replayed_claude_screen_recovers_status_without_a_hook() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal actor should start");
    actor
        .feed(
            "\u{1b}]0;current session\u{7}\u{1b}[2J\u{1b}[H────────────────────────\n❯ continue\n────────────────────────\n  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents"
                .as_bytes()
                .to_vec(),
        )
        .expect("terminal should accept Claude chrome");
    let snapshot = actor.snapshot().expect("snapshot should render");
    actor.shutdown().expect("terminal actor should stop");

    let runtime = app.terminals.get_mut(&pane_id).expect("terminal runtime");
    runtime.session = None;
    runtime.snapshot = Some(snapshot);
    app.agent_statuses.clear();
    app.poll_terminal();

    let status = app
        .agent_statuses
        .get(&pane_id)
        .expect("replayed Claude chrome should restore agent status");
    assert_eq!(status.agent, "claude");
    assert_eq!(status.state, AgentState::Idle);
}

/// A Claude Code frame mid-turn, as painted by 2.1.235: the harness
/// keeps its empty composer under the loading footer while it works.
fn claude_working_snapshot(title: &str) -> GridSnapshot {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 100,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal actor should start");
    actor
        .feed(
            format!(
                "\u{1b}]0;{title}\u{7}\u{1b}[2J\u{1b}[H✶ Sock-hopping… (1m 28s · ↓ 4.9k tokens)\n\n────────────────────────\n❯\n────────────────────────\n  ⏵⏵ auto mode on (shift+tab to cycle) · esc to interrupt · ← for agents"
            )
            .into_bytes(),
        )
        .expect("terminal should accept Claude chrome");
    let snapshot = actor.snapshot().expect("snapshot should render");
    actor.shutdown().expect("terminal actor should stop");
    snapshot
}

#[test]
fn a_pane_hands_over_to_the_agent_its_screen_belongs_to() {
    // The pane was opened for Pi — a worktree launched with the default
    // agent, or an identity restored from a persisted session — and Claude
    // Code was started in it later. Pi's rules cannot read Claude's
    // frames and Claude's hooks were refused as another agent's strays,
    // so the row stayed on Idle for the whole session.
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "pi".into(),
            display_name: Some("pi-2".into()),
            state: AgentState::Idle,
            activity: Some("Ready for input".into()),
            session_id: Some("pi-session".into()),
            cwd: Some("/work/pi-2".into()),
            git_branch: Some("pi-2".into()),
        },
    );
    app.pi_active_lifecycles.insert(pane_id);
    let runtime = app.terminals.get_mut(&pane_id).expect("terminal runtime");
    runtime.session = None;
    runtime.snapshot = Some(claude_working_snapshot(
        "◑ Fleet sidebar Running to Idle transition",
    ));
    runtime.snapshot_revision += 1;

    app.poll_terminal();

    let status = &app.agent_statuses[&pane_id];
    assert_eq!(status.agent, "claude");
    assert_eq!(status.state, AgentState::Running);
    assert_eq!(
        status.display_name.as_deref(),
        Some("Fleet sidebar Running to Idle transition"),
        "the row takes the new agent's title, without its progress glyph"
    );
    assert_eq!(
        status.session_id, None,
        "the old agent's session does not carry over"
    );
    assert_eq!(
        status.cwd.as_deref(),
        Some("/work/pi-2"),
        "pane context stays"
    );
    assert!(!app.pi_active_lifecycles.contains(&pane_id));

    // The new agent's own lifecycle is accepted from here on.
    let stopped = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "claude".into(),
        state: AgentState::Completed,
        event: Some("Stop".into()),
        title: "Claude Code · Stop".into(),
        body: "Turn complete".into(),
        pane_id: Some(pane_id.as_uuid().to_string()),
        session_id: Some("claude-session".into()),
        cwd: Some("/work/pi-2".into()),
    });
    assert!(stopped.ok);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Completed);
    assert_eq!(
        app.agent_statuses[&pane_id].session_id.as_deref(),
        Some("claude-session")
    );
}

#[test]
fn a_working_claude_frame_without_a_title_spinner_stays_running() {
    // The title prefix is optional harness chrome; the loading footer is
    // what the working frame always carries.
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let runtime = app.terminals.get_mut(&pane_id).expect("terminal runtime");
    runtime.session = None;
    runtime.snapshot = Some(claude_working_snapshot("Fleet sidebar Running to Idle"));
    runtime.snapshot_revision += 1;
    app.agent_statuses.clear();
    app.poll_terminal();
    let status = &app.agent_statuses[&pane_id];
    assert_eq!(status.agent, "claude");
    assert_eq!(status.state, AgentState::Running);
}

#[test]
fn a_working_claude_frame_with_a_status_line_and_no_title_stays_running() {
    // A configured status line replaces the footer hints and the title
    // may carry nothing at all; the progress line above the composer is
    // still there, and it is enough.
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 100,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal actor should start");
    actor
        .feed(
            "\u{1b}[2J\u{1b}[H· Bunning… (1m 44s · ↓ 3.6k tokens)\n\n────────────────────────\n❯\n────────────────────────\n  main ⎇ · 61% ctx · ← for agents"
                .as_bytes()
                .to_vec(),
        )
        .expect("terminal should accept Claude chrome");
    let snapshot = actor.snapshot().expect("snapshot should render");
    actor.shutdown().expect("terminal actor should stop");
    let runtime = app.terminals.get_mut(&pane_id).expect("terminal runtime");
    runtime.session = None;
    runtime.snapshot = Some(snapshot);
    runtime.snapshot_revision += 1;
    app.agent_statuses.clear();
    app.poll_terminal();
    let status = &app.agent_statuses[&pane_id];
    assert_eq!(status.agent, "claude");
    assert_eq!(status.state, AgentState::Running);
}

#[test]
fn a_nested_tool_run_never_relabels_a_live_agent_pane() {
    // Claude Code's own chrome stays on screen while a tool inside it
    // prints another harness's prompt glyph; the pane is still Claude's.
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Running,
            activity: None,
            session_id: Some("claude-session".into()),
            cwd: None,
            git_branch: None,
        },
    );
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 100,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal actor should start");
    actor
        .feed(
            "\u{1b}]0;◐ Probe codex\u{7}\u{1b}[2J\u{1b}[H● Bash(codex exec)\n  ⎿  › \n\n────────────────────────\n❯\n────────────────────────\n  ⏵⏵ auto mode on (shift+tab to cycle) · esc to interrupt · ← for agents"
                .as_bytes()
                .to_vec(),
        )
        .expect("terminal should accept Claude chrome");
    let snapshot = actor.snapshot().expect("snapshot should render");
    actor.shutdown().expect("terminal actor should stop");
    let runtime = app.terminals.get_mut(&pane_id).expect("terminal runtime");
    runtime.session = None;
    runtime.snapshot = Some(snapshot);
    runtime.snapshot_revision += 1;

    app.poll_terminal();

    let status = &app.agent_statuses[&pane_id];
    assert_eq!(status.agent, "claude");
    assert_eq!(status.session_id.as_deref(), Some("claude-session"));
    assert_eq!(status.state, AgentState::Running);
}

#[test]
fn harness_terminal_titles_ignore_brand_only_values_and_normalize_copy() {
    assert_eq!(harness_terminal_title("Codex", "codex"), None);
    assert_eq!(harness_terminal_title("Claude Code", "claude"), None);
    assert_eq!(harness_terminal_title("⠋ Codex", "codex"), None);
    assert_eq!(harness_terminal_title("◐ Claude Code", "claude"), None);
    assert_eq!(
        harness_terminal_title("  Fix   agent names  ", "codex"),
        Some("Fix agent names".into())
    );
    assert_eq!(
        harness_terminal_title("⠋ Fix window titles", "codex"),
        harness_terminal_title("⠙ Fix window titles", "codex")
    );
}

#[cfg(unix)]
#[test]
fn clipboard_paste_reaches_the_focused_terminal() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let _ = app.update(Message::ClipboardPasted(
        pane_id,
        Some("paste-marker".into()),
    ));

    let paste_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut text = String::new();
    while std::time::Instant::now() < paste_deadline {
        app.poll_terminal();
        text = app
            .terminals
            .get(&pane_id)
            .and_then(|runtime| runtime.snapshot.as_ref())
            .map(GridSnapshot::text)
            .unwrap_or_default();
        if text.contains("paste-marker") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        text.contains("paste-marker"),
        "pasted clipboard text did not reach the terminal grid"
    );
}

#[test]
fn clipboard_shortcuts_match_ghostty_defaults() {
    let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
    assert_eq!(
        clipboard_shortcut_for(Key::Character("C"), ctrl_shift, false),
        Some(ClipboardAction::Copy)
    );
    assert_eq!(
        clipboard_shortcut_for(Key::Character("V"), ctrl_shift, false),
        Some(ClipboardAction::Paste)
    );
    assert_eq!(
        clipboard_shortcut_for(Key::Character("c"), Modifiers::LOGO, true),
        Some(ClipboardAction::Copy)
    );
    assert_eq!(
        clipboard_shortcut_for(Key::Character("v"), Modifiers::LOGO, true),
        Some(ClipboardAction::Paste)
    );

    // Bare Ctrl+C/Ctrl+V and unrelated chords stay terminal input.
    assert_eq!(
        clipboard_shortcut_for(Key::Character("c"), Modifiers::CTRL, false),
        None
    );
    assert_eq!(
        clipboard_shortcut_for(Key::Character("v"), Modifiers::CTRL, false),
        None
    );
    assert_eq!(
        clipboard_shortcut_for(Key::Character("x"), ctrl_shift, false),
        None
    );
    assert_eq!(
        clipboard_shortcut_for(
            Key::Character("v"),
            Modifiers::LOGO | Modifiers::SHIFT,
            true
        ),
        None
    );
}

#[test]
fn ctrl_enter_extends_an_agent_prompt_instead_of_submitting() {
    assert_eq!(
        encode_terminal_key(Key::Named(Named::Enter), Modifiers::CTRL, None),
        Some(vec![b'\n'])
    );
    assert_eq!(
        encode_terminal_key(Key::Named(Named::Enter), Modifiers::empty(), None),
        Some(vec![b'\r'])
    );
}

#[test]
fn alt_arrows_walk_split_geometry_before_wrapping() {
    let profile_id = ProfileId::new();
    let mut tab = WorkspaceTab::new("Tab 1", terminal_surface(profile_id, "left"));
    let left = tab.focused_pane_id;
    let right = tab
        .split_focused(
            SplitAxis::Horizontal,
            SplitRatio::EQUAL,
            terminal_surface(profile_id, "right"),
        )
        .expect("split should succeed");
    let bottom_right = tab
        .split_focused(
            SplitAxis::Vertical,
            SplitRatio::EQUAL,
            terminal_surface(profile_id, "bottom right"),
        )
        .expect("split should succeed");

    let rects = pane_rects(&tab.root);
    assert_eq!(
        neighbor_pane(&rects, left, NavDirection::Right),
        Some(right)
    );
    assert_eq!(neighbor_pane(&rects, right, NavDirection::Left), Some(left));
    assert_eq!(
        neighbor_pane(&rects, right, NavDirection::Down),
        Some(bottom_right)
    );
    assert_eq!(
        neighbor_pane(&rects, bottom_right, NavDirection::Up),
        Some(right)
    );
    // The layout edge has no neighbor; tab wrapping takes over from here.
    assert_eq!(neighbor_pane(&rects, right, NavDirection::Right), None);
    assert_eq!(neighbor_pane(&rects, left, NavDirection::Left), None);
    assert_eq!(neighbor_pane(&rects, left, NavDirection::Up), None);
}

#[test]
fn zellij_default_tiled_layouts_preserve_pane_order() {
    let pane_ids: Vec<_> = (0..6).map(|_| PaneId::new()).collect();
    let vertical = pane_layout_tree(PaneLayout::Vertical, &pane_ids);
    let horizontal = pane_layout_tree(PaneLayout::Horizontal, &pane_ids);
    let stacked = pane_layout_tree(PaneLayout::Stacked, &pane_ids);
    let half_stacked = pane_layout_tree(PaneLayout::HalfStacked, &pane_ids);

    assert!(matches!(
        &vertical,
        PaneTree::Split {
            axis: SplitAxis::Horizontal,
            ..
        }
    ));
    assert!(matches!(
        &horizontal,
        PaneTree::Split {
            axis: SplitAxis::Vertical,
            ..
        }
    ));
    assert!(matches!(&stacked, PaneTree::Stack { .. }));
    assert!(matches!(
        &half_stacked,
        PaneTree::Split {
            axis: SplitAxis::Horizontal,
            second,
            ..
        } if matches!(second.as_ref(), PaneTree::Stack { .. })
    ));
    for layout in [vertical, horizontal, stacked, half_stacked] {
        assert_eq!(layout.pane_ids(), pane_ids);
    }

    let three = pane_layout_tree(PaneLayout::Vertical, &pane_ids[..3]);
    assert!(matches!(
        three,
        PaneTree::Split {
            axis: SplitAxis::Horizontal,
            second,
            ..
        } if matches!(
            second.as_ref(),
            PaneTree::Split {
                axis: SplitAxis::Vertical,
                ..
            }
        )
    ));
}

#[test]
fn stacked_resize_grows_then_collects_blocked_neighbors() {
    let focused = PaneId::new();
    let neighbor = PaneId::new();
    let mut tree = PaneTree::Split {
        axis: SplitAxis::Horizontal,
        ratio: SplitRatio::EQUAL,
        first: Box::new(PaneTree::leaf(focused)),
        second: Box::new(PaneTree::leaf(neighbor)),
    };

    assert!(enlarge_focused_tree(&mut tree, focused));
    assert_eq!(
        split_ratio_at(&tree, &[])
            .expect("split remains")
            .permille(),
        800
    );
    assert!(enlarge_focused_tree(&mut tree, focused));
    assert!(matches!(tree, PaneTree::Stack { .. }));
    assert_eq!(tree.pane_ids(), vec![focused, neighbor]);
}

#[test]
fn zellij_resize_prefers_the_aligned_pane_above_bottom_left() {
    let top_left = PaneId::new();
    let top_right = PaneId::new();
    let bottom_left = PaneId::new();
    let bottom_right = PaneId::new();
    let row = |first, second| PaneTree::Split {
        axis: SplitAxis::Horizontal,
        ratio: SplitRatio::EQUAL,
        first: Box::new(PaneTree::leaf(first)),
        second: Box::new(PaneTree::leaf(second)),
    };
    let mut tree = PaneTree::Split {
        axis: SplitAxis::Vertical,
        ratio: SplitRatio::EQUAL,
        first: Box::new(row(top_left, top_right)),
        second: Box::new(row(bottom_left, bottom_right)),
    };

    let direction = zellij_resize_direction(&pane_rects(&tree), bottom_left);
    assert_eq!(direction, Some(NavDirection::Up));
    assert!(enlarge_focused_tree_toward(
        &mut tree,
        bottom_left,
        direction.expect("the aligned pane above determines the direction")
    ));
    assert_eq!(
        split_ratio_at(&tree, &[])
            .expect("root split remains")
            .permille(),
        200,
        "the shared horizontal boundary should move up past halfway"
    );
}

#[test]
fn zellij_resize_ignores_an_above_pane_that_does_not_line_up() {
    let above = PaneId::new();
    let bottom_left = PaneId::new();
    let bottom_right = PaneId::new();
    let tree = PaneTree::Split {
        axis: SplitAxis::Vertical,
        ratio: SplitRatio::EQUAL,
        first: Box::new(PaneTree::leaf(above)),
        second: Box::new(PaneTree::Split {
            axis: SplitAxis::Horizontal,
            ratio: SplitRatio::EQUAL,
            first: Box::new(PaneTree::leaf(bottom_left)),
            second: Box::new(PaneTree::leaf(bottom_right)),
        }),
    };

    assert_eq!(
        zellij_resize_direction(&pane_rects(&tree), bottom_left),
        Some(NavDirection::Right),
        "a wider pane above must not override the original local split"
    );
}

#[test]
fn stacked_headers_are_keyboard_navigable_in_order() {
    let pane_ids: Vec<_> = (0..3).map(|_| PaneId::new()).collect();
    let tree = PaneTree::stack(pane_ids.clone()).expect("three panes form a stack");
    assert_eq!(
        stacked_neighbor(&tree, pane_ids[1], NavDirection::Up),
        Some(pane_ids[0])
    );
    assert_eq!(
        stacked_neighbor(&tree, pane_ids[1], NavDirection::Down),
        Some(pane_ids[2])
    );
    assert_eq!(stacked_neighbor(&tree, pane_ids[0], NavDirection::Up), None);
}

#[test]
fn half_stacked_layout_keeps_a_body_open_when_focus_is_in_the_sibling() {
    let pane_ids: Vec<_> = (0..3).map(|_| PaneId::new()).collect();
    let tree = pane_layout_tree(PaneLayout::HalfStacked, &pane_ids);
    let PaneTree::Split { second, .. } = tree else {
        panic!("half-stacked layout should have two branches");
    };
    let PaneTree::Stack { pane_ids: stack } = second.as_ref() else {
        panic!("half-stacked layout should stack the right branch");
    };

    assert_eq!(expanded_stack_pane(stack, pane_ids[0]), Some(pane_ids[1]));
    assert_eq!(expanded_stack_pane(stack, pane_ids[2]), Some(pane_ids[2]));
}

#[test]
fn layout_cycle_returns_to_the_exact_base_tree() {
    let mut app = Muxtrix::new();
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane should open");
    app.split_terminal(SplitAxis::Vertical)
        .expect("third pane should open");
    let base = active_tab(&app).root.clone();
    let left = base.pane_ids()[0];
    app.focus_pane(left).expect("left pane should focus");

    for expected in ["Vertical", "Horizontal", "Stacked", "Half-stacked", "Base"] {
        assert_eq!(
            app.cycle_pane_layout(LayoutCycle::Next)
                .expect("layout should cycle"),
            expected
        );
        assert_eq!(active_tab(&app).root.pane_ids().len(), 3);
    }
    assert_eq!(active_tab(&app).root, base);
}

#[test]
fn layout_cycle_recovers_a_pane_missing_from_the_projection() {
    let mut app = Muxtrix::new();
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane should open");
    app.split_terminal(SplitAxis::Vertical)
        .expect("third pane should open");
    let all_panes = active_tab(&app).root.pane_ids();
    active_tab_mut(&mut app).root = PaneTree::Split {
        axis: SplitAxis::Horizontal,
        ratio: SplitRatio::EQUAL,
        first: Box::new(PaneTree::leaf(all_panes[0])),
        second: Box::new(PaneTree::leaf(all_panes[1])),
    };

    assert_eq!(active_tab(&app).panes.len(), 3);
    assert_eq!(active_tab(&app).root.pane_ids().len(), 2);
    let recovered_order = pane_ids_for_layout(active_tab(&app));

    for expected in ["Vertical", "Horizontal", "Stacked", "Half-stacked", "Base"] {
        assert_eq!(
            app.cycle_pane_layout(LayoutCycle::Next)
                .expect("layout should cycle"),
            expected
        );
        assert_eq!(active_tab(&app).root.pane_ids(), recovered_order);
    }
    assert_eq!(
        active_tab(&app).root,
        pane_layout_tree(PaneLayout::Vertical, &recovered_order)
    );
}

#[test]
fn pane_resize_decrease_walks_the_undo_chain() {
    let mut app = Muxtrix::new();
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane should open");
    let base = active_tab(&app).root.clone();

    app.resize_focused_pane(true)
        .expect("first resize should adjust the split");
    app.resize_focused_pane(true)
        .expect("second resize should stack blocked panes");
    assert!(matches!(active_tab(&app).root, PaneTree::Stack { .. }));
    app.resize_focused_pane(false)
        .expect("first decrease should restore the split");
    app.resize_focused_pane(false)
        .expect("second decrease should restore the base");
    assert_eq!(active_tab(&app).root, base);
}

#[test]
fn pane_resize_decrease_moves_the_chosen_boundary_back_after_crossing_halfway() {
    let mut app = Muxtrix::new();
    let top_left = active_pane_id(&app);
    app.split_terminal(SplitAxis::Horizontal)
        .expect("top-right pane should open");
    let top_right = active_pane_id(&app);
    app.focus_pane(top_left)
        .expect("top-left pane should focus");
    app.split_terminal(SplitAxis::Vertical)
        .expect("bottom-left pane should open");
    let bottom_left = active_pane_id(&app);
    app.focus_pane(top_right)
        .expect("top-right pane should focus");
    app.split_terminal(SplitAxis::Vertical)
        .expect("bottom-right pane should open");
    let bottom_right = active_pane_id(&app);
    active_tab_mut(&mut app).root = PaneTree::Split {
        axis: SplitAxis::Vertical,
        ratio: SplitRatio::EQUAL,
        first: Box::new(PaneTree::Split {
            axis: SplitAxis::Horizontal,
            ratio: SplitRatio::EQUAL,
            first: Box::new(PaneTree::leaf(top_left)),
            second: Box::new(PaneTree::leaf(top_right)),
        }),
        second: Box::new(PaneTree::Split {
            axis: SplitAxis::Horizontal,
            ratio: SplitRatio::EQUAL,
            first: Box::new(PaneTree::leaf(bottom_left)),
            second: Box::new(PaneTree::leaf(bottom_right)),
        }),
    };
    app.focus_pane(bottom_left)
        .expect("bottom-left pane should focus");
    let base = active_tab(&app).root.clone();

    app.resize_focused_pane(true)
        .expect("grow should move the aligned boundary up");
    assert_eq!(
        split_ratio_at(&active_tab(&app).root, &[])
            .expect("root split remains")
            .permille(),
        200
    );
    app.resize_focused_pane(false)
        .expect("decrease should move that boundary back down");
    assert_eq!(active_tab(&app).root, base);
}

#[test]
fn edge_navigation_wraps_to_the_neighboring_tab() {
    let mut app = Muxtrix::new();
    let first_pane = active_pane_id(&app);
    app.new_tab().expect("second tab should open");
    let second_pane = active_pane_id(&app);
    assert_ne!(first_pane, second_pane);

    app.focus_neighbor_pane(NavDirection::Right)
        .expect("navigation should wrap forward");
    assert_eq!(active_pane_id(&app), first_pane);
    app.focus_neighbor_pane(NavDirection::Left)
        .expect("navigation should wrap backward");
    assert_eq!(active_pane_id(&app), second_pane);
}

#[cfg(target_os = "linux")]
#[test]
fn agent_status_returns_to_shell_after_hooked_process_exits() {
    let mut app = Muxtrix::new();
    // Any process whose comm matches the configured agent executable
    // counts; `sleep` stands in for the codex binary here.
    app.settings.codex_command = "sleep 30".into();
    let pane_id = active_pane_id(&app);

    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Launched indirectly (as via shell history in the real report), so
    // the typed-command detector cannot see it — only process detection.
    app.send_terminal_input(b"sh -c 'exec sleep 30'\r".to_vec())
        .expect("shell should accept the command");

    let detect_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < detect_deadline {
        app.detect_agent_processes();
        if app.agent_statuses.contains_key(&pane_id) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let status = app
        .agent_statuses
        .get(&pane_id)
        .expect("a running agent process should be detected without hooks");
    assert_eq!(status.agent, "codex");
    assert_eq!(status.state, AgentState::Running);
    assert!(app.detected_agents.contains_key(&pane_id));

    // Once a lifecycle hook arrives, process observation must remain in
    // place so leaving the harness can restore truthful shell state even
    // when its final hook is missing.
    let hooked = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("UserPromptSubmit".into()),
        title: "Codex · UserPromptSubmit".into(),
        body: "Working".into(),
        pane_id: Some(pane_id.as_uuid().to_string()),
        session_id: Some("thread-1".into()),
        cwd: None,
    });
    assert!(hooked.ok);
    // Model a hook arriving before the periodic process scan saw the
    // harness: an existing hook-owned status must still join observation.
    app.detected_agents.remove(&pane_id);
    app.detect_agent_processes();
    assert!(
        app.detected_agents.contains_key(&pane_id),
        "hook-owned statuses must retain process-exit observation"
    );
    app.detected_agents.insert(
        pane_id,
        std::time::Instant::now() - std::time::Duration::from_secs(3),
    );
    app.send_terminal_input(vec![0x03]).expect("ctrl+c");
    let gone_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < gone_deadline {
        app.detect_agent_processes();
        if !app.agent_statuses.contains_key(&pane_id) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        !app.agent_statuses.contains_key(&pane_id),
        "a detected status must self-clean when its process exits"
    );
    assert_eq!(app.pane_state_label(pane_id), "Shell");
}

#[test]
fn worktree_names_are_safe_for_branches_and_directories() {
    assert_eq!(worktree_name("  fix login bug  "), "fix-login-bug");
    assert_eq!(worktree_name("feature/palette"), "feature-palette");
    assert_eq!(worktree_name("../escape"), "escape");
    assert_eq!(worktree_name("   "), "");
}

#[cfg(target_os = "linux")]
#[test]
fn clean_exit_cascades_in_daemon_mode() {
    // The regression class this pins: local-PTY panes cascaded on clean
    // exit, daemon-owned panes silently stopped.
    let id = uuid::Uuid::new_v4();
    let endpoint = muxtrix_sessions::session_endpoint(id);
    let daemon_endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let _ = muxtrix_sessions::daemon::run(id, "cascade-test".into(), daemon_endpoint);
    });
    assert!(muxtrix_sessions::daemon::wait_until_ready(&endpoint));
    let (client, _, _) =
        muxtrix_sessions::SessionClient::connect_endpoint(&endpoint).expect("attach");
    let client = Arc::new(client);
    let mut app = Muxtrix::new();
    app.terminal_launcher = Arc::new(SystemTerminalLauncher {
        client: Some(Arc::clone(&client)),
    });
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane");
    let pane_id = active_pane_id(&app);
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let pane_count = |app: &Muxtrix| {
        app.session
            .workspaces
            .iter()
            .map(muxtrix_domain::Workspace::pane_count)
            .sum::<usize>()
    };
    assert_eq!(pane_count(&app), 2);
    app.send_terminal_input(b"exit 0\r".to_vec())
        .expect("shell should accept exit");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        if pane_count(&app) == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        pane_count(&app),
        1,
        "a cleanly exited daemon pane must close and cascade"
    );
    let _ = client.send(&muxtrix_sessions::Request::Shutdown);
    // Test daemons must not leave records behind: the session picker
    // reads that directory.
    muxtrix_sessions::remove_session_record(id);
}

#[cfg(target_os = "linux")]
#[test]
fn closing_a_daemon_pane_releases_the_byte_channel_it_was_read_through() {
    // The leak this pins: the daemon reports no exit for a pane it was
    // told to kill, so nothing closed the pane's output channel and its
    // reader thread blocked on an empty receiver for the life of the app.
    let id = uuid::Uuid::new_v4();
    let endpoint = muxtrix_sessions::session_endpoint(id);
    let daemon_endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let _ = muxtrix_sessions::daemon::run(id, "close-test".into(), daemon_endpoint);
    });
    assert!(muxtrix_sessions::daemon::wait_until_ready(&endpoint));
    let (client, _, _) =
        muxtrix_sessions::SessionClient::connect_endpoint(&endpoint).expect("attach");
    let client = Arc::new(client);
    let mut app = Muxtrix::new();
    app.terminal_launcher = Arc::new(SystemTerminalLauncher {
        client: Some(Arc::clone(&client)),
    });
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane");
    let pane_id = active_pane_id(&app);
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        client.tracks_pane(pane_id.as_uuid()),
        "the pane should be streaming before it is closed"
    );

    app.close_pane(pane_id).expect("close the pane");

    // The kill travels through the pane's session thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline && client.tracks_pane(pane_id.as_uuid()) {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !client.tracks_pane(pane_id.as_uuid()),
        "closing a pane must release its byte channel, or its reader never ends"
    );
    let _ = client.send(&muxtrix_sessions::Request::Shutdown);
    // Test daemons must not leave records behind: the session picker
    // reads that directory.
    muxtrix_sessions::remove_session_record(id);
}

/// Restarting a pane in a worktree reuses the pane's durable identity
/// against the real daemon, on the real background launch path. The
/// replacement must become a live, streaming terminal.
#[cfg(target_os = "linux")]
#[test]
fn restarting_a_daemon_pane_in_a_directory_streams_its_replacement() {
    let id = uuid::Uuid::new_v4();
    let endpoint = muxtrix_sessions::session_endpoint(id);
    let daemon_endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let _ = muxtrix_sessions::daemon::run(id, "restart-test".into(), daemon_endpoint);
    });
    assert!(muxtrix_sessions::daemon::wait_until_ready(&endpoint));
    let (client, _, _) =
        muxtrix_sessions::SessionClient::connect_endpoint(&endpoint).expect("attach");
    let client = Arc::new(client);
    let mut app = Muxtrix::new();
    app.terminal_launcher = Arc::new(SystemTerminalLauncher {
        client: Some(Arc::clone(&client)),
    });
    // Production always launches off the UI thread.
    app.launch_in_background = true;
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane");
    let pane_id = active_pane_id(&app);

    let settle = |app: &mut Muxtrix, marker: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            app.poll_terminal();
            if app.terminals[&pane_id]
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.text().contains(marker))
            {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    };
    let run_marker = |app: &mut Muxtrix, marker: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            app.poll_terminal();
            if matches!(
                app.terminals[&pane_id].launch_state,
                TerminalLaunchState::Running
            ) && app
                .send_terminal_input_to(pane_id, format!("printf {marker}\r").into_bytes())
                .is_ok()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };

    run_marker(&mut app, "first-shell");
    assert!(
        settle(&mut app, "first-shell"),
        "the pane should stream before it is restarted"
    );

    app.restart_pane_in_directory(pane_id, std::env::temp_dir())
        .expect("restart in the requested directory");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        if !matches!(
            app.terminals[&pane_id].launch_state,
            TerminalLaunchState::Starting { .. }
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        matches!(
            app.terminals[&pane_id].launch_state,
            TerminalLaunchState::Running
        ),
        "the restarted pane never became live: {:?}",
        app.terminals[&pane_id].launch_state
    );

    run_marker(&mut app, "restarted-shell");
    assert!(
        settle(&mut app, "restarted-shell"),
        "the restarted pane never streamed its own output"
    );
    assert!(
        client.tracks_pane(pane_id.as_uuid()),
        "the replacement's byte channel must still be open"
    );
    let _ = client.send(&muxtrix_sessions::Request::Shutdown);
    // Test daemons must not leave records behind: the session picker
    // reads that directory.
    muxtrix_sessions::remove_session_record(id);
}

/// The same restart, taken while the pane's first launch is still in
/// flight — the fresh-tab-then-pick-a-worktree sequence.
#[cfg(target_os = "linux")]
#[test]
fn restarting_a_daemon_pane_mid_launch_streams_its_replacement() {
    let id = uuid::Uuid::new_v4();
    let endpoint = muxtrix_sessions::session_endpoint(id);
    let daemon_endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let _ = muxtrix_sessions::daemon::run(id, "queued-restart-test".into(), daemon_endpoint);
    });
    assert!(muxtrix_sessions::daemon::wait_until_ready(&endpoint));
    let (client, _, _) =
        muxtrix_sessions::SessionClient::connect_endpoint(&endpoint).expect("attach");
    let client = Arc::new(client);
    let mut app = Muxtrix::new();
    app.terminal_launcher = Arc::new(SystemTerminalLauncher {
        client: Some(Arc::clone(&client)),
    });
    app.launch_in_background = true;

    app.new_tab().expect("fresh tab");
    let pane_id = active_pane_id(&app);
    // No poll in between: the first launch is still on its worker.
    app.restart_pane_in_directory(pane_id, std::env::temp_dir())
        .expect("restart into the worktree directory");
    assert!(
        app.queued_terminal_restarts.contains(&pane_id),
        "the restart should have queued behind the first launch"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        if matches!(
            app.terminals[&pane_id].launch_state,
            TerminalLaunchState::Running
        ) && app.queued_terminal_restarts.is_empty()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        matches!(
            app.terminals[&pane_id].launch_state,
            TerminalLaunchState::Running
        ),
        "the restarted pane never became live: {:?}",
        app.terminals[&pane_id].launch_state
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut streamed = false;
    while std::time::Instant::now() < deadline && !streamed {
        app.poll_terminal();
        let _ = app.send_terminal_input_to(pane_id, b"printf queued-marker\r".to_vec());
        for _ in 0..25 {
            app.poll_terminal();
            streamed = app.terminals[&pane_id]
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.text().contains("queued-marker"));
            if streamed {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    assert!(
        streamed,
        "the replacement pane never streamed: it is a live-looking terminal that \
         receives no bytes"
    );
    assert_eq!(
        app.active_workspace()
            .expect("workspace")
            .pane(pane_id)
            .and_then(Pane::active_surface)
            .and_then(|surface| match &surface.kind {
                muxtrix_domain::SurfaceKind::Terminal(terminal) =>
                    terminal.working_directory.clone(),
                _ => None,
            })
            .as_deref(),
        Some(std::env::temp_dir().as_path()),
        "the hand-off must keep the directory the restart was chosen for"
    );
    let _ = client.send(&muxtrix_sessions::Request::Shutdown);
    // Test daemons must not leave records behind: the session picker
    // reads that directory.
    muxtrix_sessions::remove_session_record(id);
}

/// A pane the host refuses to start must say so in place. The daemon
/// accepts the spawn request and reports the refusal afterwards, so
/// without this the pane renders as a live terminal that never prints.
#[cfg(target_os = "linux")]
#[test]
fn a_pane_the_host_cannot_start_reports_why_instead_of_staying_blank() {
    let id = uuid::Uuid::new_v4();
    let endpoint = muxtrix_sessions::session_endpoint(id);
    let daemon_endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let _ = muxtrix_sessions::daemon::run(id, "refusal-test".into(), daemon_endpoint);
    });
    assert!(muxtrix_sessions::daemon::wait_until_ready(&endpoint));
    let (client, _, _) =
        muxtrix_sessions::SessionClient::connect_endpoint(&endpoint).expect("attach");
    let client = Arc::new(client);
    let mut app = Muxtrix::new();
    app.terminal_launcher = Arc::new(SystemTerminalLauncher {
        client: Some(Arc::clone(&client)),
    });
    app.launch_in_background = true;
    let missing = std::env::temp_dir().join("muxtrix-shell-that-does-not-exist");
    for profile in &mut app.session.profiles {
        profile.backend = ProcessBackend::Local;
        profile.program = missing.to_string_lossy().into_owned();
        profile.arguments.clear();
    }
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane");
    let pane_id = active_pane_id(&app);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        if matches!(
            app.terminals[&pane_id].launch_state,
            TerminalLaunchState::Failed(_)
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let TerminalLaunchState::Failed(error) = &app.terminals[&pane_id].launch_state else {
        panic!(
            "a pane the host never started must read as failed, not {:?}",
            app.terminals[&pane_id].launch_state
        );
    };
    assert!(
        error.contains("muxtrix-shell-that-does-not-exist"),
        "the pane should name what the host could not start: {error}"
    );
    let _ = client.send(&muxtrix_sessions::Request::Shutdown);
    // Test daemons must not leave records behind: the session picker
    // reads that directory.
    muxtrix_sessions::remove_session_record(id);
}

#[cfg(target_os = "linux")]
#[test]
fn injected_prompt_command_reports_pwd_through_osc7() {
    if std::process::Command::new("bash")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // The pane's sh received PROMPT_COMMAND from the launch environment;
    // exec-ing bash inherits it, and --norc/--noprofile prove the report
    // needs no rc-file cooperation. The cd must then surface through
    // snapshot.pwd — the exact chain the Windows+WSL build relies on.
    app.send_terminal_input(
        b"exec bash --noprofile --norc
"
        .to_vec(),
    )
    .expect("shell should exec bash");
    app.send_terminal_input(
        b"cd /tmp
"
        .to_vec(),
    )
    .expect("bash should accept cd");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut reported = None;
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        reported = app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.pwd.as_deref())
            .and_then(decode_reported_pwd)
            .filter(|path| path == std::path::Path::new("/tmp"));
        if reported.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        reported,
        Some(std::path::PathBuf::from("/tmp")),
        "bash with injected PROMPT_COMMAND must report its pwd via OSC 7; \
         snapshot pwd was {:?}",
        app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.pwd.clone())
    );
}

/// Waits for the pane's shell to report `expected` via OSC 7 after the
/// given inputs run — the exact chain WSL-hosted panes depend on.
#[cfg(target_os = "linux")]
fn assert_shell_reports_pwd(shell_command: &str, expected: &str) {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.send_terminal_input(format!("exec {shell_command}\r").into_bytes())
        .expect("shell should exec");
    // Wait for the new shell's first prompt-time report before typing:
    // input sent during exec can still be sitting in the old shell's
    // read buffer and would vanish with it.
    let first_report_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < first_report_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.pwd.is_some())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    app.send_terminal_input(format!("cd {expected}\r").into_bytes())
        .expect("shell should accept cd");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut reported = None;
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        reported = app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.pwd.as_deref())
            .and_then(decode_reported_pwd)
            .filter(|path| path == std::path::Path::new(expected));
        if reported.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        reported,
        Some(std::path::PathBuf::from(expected)),
        "{shell_command} must report its pwd via OSC 7; snapshot pwd was {:?}; screen: {:?}",
        app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.pwd.clone()),
        app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.text())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn staged_fish_integration_reports_pwd_through_osc7() {
    if std::process::Command::new("fish")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    // fish only volunteers OSC 7 to allowlisted terminals; the staged
    // conf.d snippet (gated on MUXTRIX_PANE_ID) must cover ours.
    assert_shell_reports_pwd("fish", "/tmp");
}

#[cfg(target_os = "linux")]
#[test]
fn staged_zsh_bridge_reports_pwd_through_osc7() {
    if std::process::Command::new("zsh")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    // The redirected ZDOTDIR bridge installs the precmd hook and then
    // hands control back to the user's real dotfiles. The bridge is
    // re-pointed explicitly because the pane's own shell may be zsh
    // already — its bridge run restores ZDOTDIR, so a nested zsh would
    // otherwise start without one (same limitation Ghostty has).
    assert_shell_reports_pwd(
        "env ZDOTDIR=\"$HOME/.local/share/muxtrix/shell-integration/zsh\" zsh",
        "/tmp",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn focused_pane_detects_the_repository_it_runs_in() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    // Panes start in $HOME; the user flow is cd-ing into a repository
    // first, so the test does the same with this crate's own repo.
    let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < ready_deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.text().trim().is_empty())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.send_terminal_input(format!("cd {}\r", env!("CARGO_MANIFEST_DIR")).into_bytes())
        .expect("shell should accept cd");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut root = None;
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        root = app
            .pane_working_directory(pane_id)
            .as_deref()
            .and_then(|directory| git_repository_root(directory, ""));
        if root.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let root = root.unwrap_or_else(|| {
        let pid = app.terminals[&pane_id]
            .session
            .as_ref()
            .and_then(LiveSession::process_id);
        let cwd = app.pane_working_directory(pane_id);
        panic!("no repository detected: pid={pid:?} cwd={cwd:?}")
    });
    assert!(root.join(".git").exists());
    let _ = pane_id;
}

#[test]
fn reported_pwd_decoding_handles_osc7_uris_and_bare_paths() {
    // OSC 7: file:// URI, optionally with a hostname and percent-encoding.
    assert_eq!(
        decode_reported_pwd("file:///home/user"),
        Some(std::path::PathBuf::from("/home/user"))
    );
    assert_eq!(
        decode_reported_pwd("file://mymachine/home/user/my%20repo"),
        Some(std::path::PathBuf::from("/home/user/my repo"))
    );
    // OSC 9/1337: bare paths pass through.
    assert_eq!(
        decode_reported_pwd("/tmp/x"),
        Some(std::path::PathBuf::from("/tmp/x"))
    );
    assert_eq!(
        decode_reported_pwd("C:\\Users\\dev"),
        Some(std::path::PathBuf::from("C:\\Users\\dev"))
    );
    assert_eq!(decode_reported_pwd(""), None);
    assert_eq!(decode_reported_pwd("file://"), None);
}

#[test]
fn default_worktree_name_skips_taken_names() {
    let taken: BTreeSet<String> = ["worktree-1", "worktree-2"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(default_worktree_name(&taken), "worktree-3");
    assert_eq!(default_worktree_name(&BTreeSet::new()), "worktree-1");
}

#[test]
fn rail_targets_walk_workspaces_then_fleet_in_visual_order() {
    let app = Muxtrix::new();
    let targets = app.rail_targets();
    let workspace_count = targets
        .iter()
        .take_while(|target| matches!(target, RailTarget::Workspace(_)))
        .count();
    assert!(workspace_count >= 1, "workspaces lead the walk");
    assert!(
        targets[workspace_count..]
            .iter()
            .all(|target| !matches!(target, RailTarget::Workspace(_))),
        "fleet entries never interleave with workspaces"
    );
    assert!(
        matches!(
            targets.get(workspace_count),
            Some(RailTarget::FleetPane(..))
        ),
        "a one-tab workspace starts at its first visible pane row"
    );
    assert!(
        targets
            .iter()
            .all(|target| !matches!(target, RailTarget::FleetTab(..))),
        "navigation never lands on a hidden single-tab band"
    );
}

#[test]
fn tabs_view_keeps_visible_tab_bands_in_the_active_workspace() {
    let mut app = Muxtrix::new();
    let workspace_id = app.session.active_workspace_id;
    let first_tab = active_tab(&app).id;
    let first_pane = active_pane_id(&app);
    app.new_tab().expect("second tab should be created");
    let second_tab = active_tab(&app).id;
    let second_pane = active_pane_id(&app);

    assert_eq!(
        app.rail_targets()
            .into_iter()
            .filter(|target| !matches!(target, RailTarget::Workspace(_)))
            .collect::<Vec<_>>(),
        vec![
            RailTarget::FleetTab(workspace_id, first_tab),
            RailTarget::FleetPane(workspace_id, first_pane),
            RailTarget::FleetTab(workspace_id, second_tab),
            RailTarget::FleetPane(workspace_id, second_pane),
        ],
        "Tabs preserves tab bands even though workspace bands are gone"
    );
}

#[test]
fn repos_view_groups_across_tabs_without_nested_tab_bands() {
    let mut app = Muxtrix::new();
    let workspace_id = app.session.active_workspace_id;
    let first_repo_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let no_repo_pane = active_pane_id(&app);
    app.new_tab().expect("second tab should be created");
    let second_repo_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let other_repo_pane = active_pane_id(&app);

    for (pane_id, name) in [
        (first_repo_pane, Some("muxtrix")),
        (no_repo_pane, None),
        (second_repo_pane, Some("muxtrix")),
        (other_repo_pane, Some("mailmatrix")),
    ] {
        let directory = app
            .pane_working_directory(pane_id)
            .expect("test pane should have a working directory");
        app.pane_repositories.insert(
            pane_id,
            PaneRepository {
                directory,
                root: None,
                name: name.map(str::to_owned),
                worktree_name: None,
                branch: None,
                reported_branch: None,
                head_oid: None,
                pull_request: None,
                checked_at: std::time::Instant::now(),
            },
        );
    }
    app.set_fleet_view(FleetView::Repos);

    assert_eq!(
        app.fleet_repository_groups(),
        vec![
            FleetRepositoryGroup {
                name: "muxtrix".into(),
                entries: vec![
                    (workspace_id, first_repo_pane),
                    (workspace_id, second_repo_pane),
                ],
            },
            FleetRepositoryGroup {
                name: "mailmatrix".into(),
                entries: vec![(workspace_id, other_repo_pane)],
            },
            FleetRepositoryGroup {
                name: NO_REPO_GROUP.into(),
                entries: vec![(workspace_id, no_repo_pane)],
            },
        ]
    );
    assert_eq!(
        app.rail_targets()
            .into_iter()
            .filter(|target| !matches!(target, RailTarget::Workspace(_)))
            .collect::<Vec<_>>(),
        vec![
            RailTarget::FleetGroup(workspace_id, first_repo_pane),
            RailTarget::FleetPane(workspace_id, first_repo_pane),
            RailTarget::FleetPane(workspace_id, second_repo_pane),
            RailTarget::FleetGroup(workspace_id, other_repo_pane),
            RailTarget::FleetPane(workspace_id, other_repo_pane),
            RailTarget::FleetGroup(workspace_id, no_repo_pane),
            RailTarget::FleetPane(workspace_id, no_repo_pane),
        ],
        "Repos uses repository bands only and preserves pane order inside each group"
    );
}

#[test]
fn all_workspace_repos_keep_identical_repository_names_in_workspace_groups() {
    let mut app = Muxtrix::new();
    let first_workspace = app.session.active_workspace_id;
    let first_pane = active_pane_id(&app);
    create_test_workspace(&mut app);
    let second_workspace = app.session.active_workspace_id;
    let second_pane = active_pane_id(&app);

    for pane_id in [first_pane, second_pane] {
        let directory = app
            .pane_working_directory(pane_id)
            .expect("test pane should have a working directory");
        app.pane_repositories.insert(
            pane_id,
            PaneRepository {
                directory,
                root: None,
                name: Some("muxtrix".into()),
                worktree_name: None,
                branch: None,
                reported_branch: None,
                head_oid: None,
                pull_request: None,
                checked_at: std::time::Instant::now(),
            },
        );
    }
    app.set_fleet_scope(FleetScope::AllWorkspaces);
    app.set_fleet_view(FleetView::Repos);

    assert_eq!(
        app.fleet_repository_groups(),
        vec![
            FleetRepositoryGroup {
                name: "muxtrix".into(),
                entries: vec![(first_workspace, first_pane)],
            },
            FleetRepositoryGroup {
                name: "muxtrix".into(),
                entries: vec![(second_workspace, second_pane)],
            },
        ],
        "repository bands must not merge across workspace boundaries"
    );
    assert_eq!(
        app.rail_targets()
            .into_iter()
            .filter(|target| !matches!(target, RailTarget::Workspace(_)))
            .collect::<Vec<_>>(),
        vec![
            RailTarget::FleetWorkspace(first_workspace),
            RailTarget::FleetGroup(first_workspace, first_pane),
            RailTarget::FleetPane(first_workspace, first_pane),
            RailTarget::FleetWorkspace(second_workspace),
            RailTarget::FleetGroup(second_workspace, second_pane),
            RailTarget::FleetPane(second_workspace, second_pane),
        ]
    );
}

#[test]
fn linked_worktrees_keep_the_primary_repository_group_name() {
    let scratch = std::env::temp_dir().join(format!(
        "muxtrix-repository-name-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    let repo = scratch.join("muxtrix-source");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("file"), "one").expect("file");
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    let linked = scratch.join("worktrees").join("feature-a");
    create_git_worktree(&repo, &linked, "feature-a", "").expect("linked worktree");

    assert_eq!(
        git_repository_name(&repo, "").as_deref(),
        Some("muxtrix-source")
    );
    assert_eq!(
        git_repository_name(&linked, "").as_deref(),
        Some("muxtrix-source"),
        "linked checkout names must not split one repository into separate groups"
    );
    let primary_base = worktree_base_directory(&repo, "").expect("primary worktree base directory");
    let linked_base = worktree_base_directory(&linked, "").expect("linked worktree base directory");
    assert_eq!(
        linked_base, primary_base,
        "all worktrees from one repository must share its namespace"
    );
    assert_eq!(
        linked_base.file_name().and_then(|name| name.to_str()),
        Some("muxtrix-source")
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn prefix_and_rail_navigation_only_exit_explicitly() {
    let mut app = Muxtrix::new();

    let _ = app.handle_keyboard(key_press(Key::Character("g".into()), Modifiers::CTRL));
    assert!(app.prefix_armed);
    assert_eq!(
        app.feedback_message(),
        Some(("Prefix — w workspaces · f fleet · Esc cancel", true)),
        "an armed prefix is a keyboard mode, not a transient toast"
    );

    let _ = app.handle_keyboard(key_press(Key::Character("x".into()), Modifiers::empty()));
    assert!(
        app.prefix_armed,
        "an unrelated key cannot cancel the prefix"
    );

    let _ = app.handle_keyboard(key_press(Key::Character("f".into()), Modifiers::empty()));
    let target = app
        .rail_nav
        .expect("fleet follow-up should start navigation");
    assert_eq!(
        app.feedback_message(),
        Some(("Navigate — ↑↓ move · Enter select · Esc exit", true)),
        "rail navigation is a keyboard mode, not a transient toast"
    );

    let _ = app.handle_keyboard(key_press(Key::Character("x".into()), Modifiers::empty()));
    assert_eq!(
        app.rail_nav,
        Some(target),
        "an unrelated key cannot cancel rail navigation"
    );

    let _ = app.handle_keyboard(key_press(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(app.rail_nav.is_none());
    assert!(app.feedback_message().is_none());
}

#[test]
fn rail_navigation_selection_exits_the_mode() {
    let mut app = Muxtrix::new();
    let _ = app.handle_keyboard(key_press(Key::Character("g".into()), Modifiers::CTRL));
    let _ = app.handle_keyboard(key_press(Key::Character("w".into()), Modifiers::empty()));
    assert!(matches!(app.rail_nav, Some(RailTarget::Workspace(_))));

    let _ = app.handle_keyboard(key_press(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.rail_nav.is_none());
    assert!(app.feedback_message().is_none());
}

#[test]
fn command_shift_number_switches_workspace_by_session_order() {
    let mut app = Muxtrix::new();
    create_test_workspace(&mut app);
    create_test_workspace(&mut app);
    let second_workspace = app.session.workspaces[1].id;
    let second_workspace_name = app.session.workspaces[1].name.clone();
    assert_ne!(app.session.active_workspace_id, second_workspace);

    let _ = app.handle_keyboard(KeyEvent::Pressed(KeyInput {
        key: Key::Character("2".into()),
        // The platform applies Shift to `modified_key`; shortcut matching
        // must use the unmodified key or Shift+2 becomes "@" on a US
        // layout.
        modified_key: Key::Character("@".into()),
        modifiers: Modifiers::COMMAND | Modifiers::SHIFT,
        text: Some("@".into()),
        repeat: false,
    }));

    assert_eq!(app.session.active_workspace_id, second_workspace);
    assert_eq!(app.workspace_name_draft, second_workspace_name);
    assert_eq!(app.active_view, ActiveView::Workspace);
}

#[test]
fn worktree_list_porcelain_parses_paths_and_branches() {
    let listing = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /wt/one\nHEAD def\nbranch refs/heads/worktree-1\n\nworktree /wt/detached\nHEAD 123\ndetached\n";
    let parsed = parse_worktree_list(listing);
    assert_eq!(
        parsed,
        vec![
            (std::path::PathBuf::from("/repo"), Some("main".into())),
            (
                std::path::PathBuf::from("/wt/one"),
                Some("worktree-1".into())
            ),
            (std::path::PathBuf::from("/wt/detached"), None),
        ]
    );
}

#[test]
fn linked_worktree_identity_uses_the_checkout_leaf_only_for_linked_trees() {
    let base =
        std::env::temp_dir().join(format!("muxtrix-agent-title-test-{}", uuid::Uuid::new_v4()));
    let worktree = base.join("a-very-long-feature-name");
    std::fs::create_dir_all(worktree.join("src")).expect("worktree fixture");
    std::fs::write(worktree.join(".git"), "gitdir: /repo/.git/worktrees/name\n")
        .expect("linked worktree marker");
    assert_eq!(
        linked_worktree_name(&worktree.join("src")),
        Some("a-very-long-feature-name".into())
    );
    assert_eq!(
        linked_worktree_name_from_convention(std::path::Path::new(
            "/home/user/.muxtrix/worktrees/muxtrix/a-very-long-feature-name/src"
        )),
        Some("a-very-long-feature-name".into())
    );
    assert_eq!(
        linked_worktree_name_from_convention(std::path::Path::new(
            "/home/user/dev/muxtrix/.claude/worktrees/fix-agent-titles/src"
        )),
        Some("fix-agent-titles".into())
    );
    assert_eq!(
        linked_worktree_name(std::path::Path::new("/repo/main")),
        None
    );
}

#[test]
fn fleet_location_prefers_worktree_then_repository_then_directory() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let directory = app
        .pane_working_directory(pane_id)
        .expect("test pane should have a working directory");
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory: directory.clone(),
            root: None,
            name: Some("muxtrix".into()),
            worktree_name: Some("fleet-two-line-rows".into()),
            branch: None,
            reported_branch: None,
            head_oid: None,
            pull_request: None,
            checked_at: std::time::Instant::now(),
        },
    );

    assert_eq!(app.pane_location_label(pane_id), "fleet-two-line-rows");
    app.pane_repositories
        .get_mut(&pane_id)
        .expect("repository metadata")
        .worktree_name = None;
    assert_eq!(app.pane_location_label(pane_id), "muxtrix");
    let repository = app
        .pane_repositories
        .get_mut(&pane_id)
        .expect("repository metadata");
    repository.name = None;
    assert_eq!(
        app.pane_location_label(pane_id),
        directory.display().to_string()
    );
}

#[test]
fn fleet_row_spends_duplicate_automatic_titles_on_command_copy() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let duplicate = "fleet-two-line-rows";
    {
        let pane = active_tab_mut(&mut app)
            .panes
            .get_mut(&pane_id)
            .expect("pane should exist");
        let surface = pane
            .surfaces
            .iter_mut()
            .find(|surface| surface.id == pane.active_surface_id)
            .expect("surface should exist");
        surface.title = duplicate.into();
    }
    let directory = app
        .pane_working_directory(pane_id)
        .expect("test pane should have a working directory");
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory,
            root: None,
            name: Some("muxtrix".into()),
            worktree_name: Some(duplicate.into()),
            branch: None,
            reported_branch: None,
            head_oid: None,
            pull_request: None,
            checked_at: std::time::Instant::now(),
        },
    );

    let location = app.pane_location_label(pane_id);
    let command = app.pane_command(pane_id);
    assert!(!command.is_empty(), "the fallback must still be truthful");
    assert_eq!(
        app.fleet_pane_identity_label(
            app.active_workspace().expect("workspace should exist"),
            pane_id,
            &location
        ),
        command
    );
}

#[test]
fn fleet_row_preserves_intentional_pane_names_even_when_repeated() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let duplicate = "fleet-two-line-rows";
    active_tab_mut(&mut app)
        .panes
        .get_mut(&pane_id)
        .expect("pane should exist")
        .custom_name = Some(duplicate.into());
    let location = duplicate.to_owned();

    assert_eq!(
        app.fleet_pane_identity_label(
            app.active_workspace().expect("workspace should exist"),
            pane_id,
            &location
        ),
        duplicate
    );
}

#[test]
fn fleet_row_spends_duplicate_agent_titles_on_activity_copy() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let duplicate = "feature-ui";
    let activity = "Rewriting the rail";
    let directory = app
        .pane_working_directory(pane_id)
        .expect("test pane should have a working directory");
    app.pane_repositories.insert(
        pane_id,
        PaneRepository {
            directory,
            root: None,
            name: Some("muxtrix".into()),
            worktree_name: Some(duplicate.into()),
            branch: None,
            reported_branch: None,
            head_oid: None,
            pull_request: None,
            checked_at: std::time::Instant::now(),
        },
    );
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: Some(duplicate.into()),
            state: AgentState::Running,
            activity: Some(activity.into()),
            session_id: None,
            cwd: None,
            git_branch: None,
        },
    );
    let location = app.pane_location_label(pane_id);

    assert_eq!(
        app.fleet_pane_identity_label(
            app.active_workspace().expect("workspace should exist"),
            pane_id,
            &location
        ),
        activity
    );
}

#[test]
fn github_default_branch_comes_from_the_github_remote_head() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let repo = std::env::temp_dir().join(format!(
        "muxtrix-default-branch-test-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&[
        "remote",
        "add",
        "upstream",
        "git@github.com:acme/project.git",
    ]);
    git(&[
        "symbolic-ref",
        "refs/remotes/upstream/HEAD",
        "refs/remotes/upstream/trunk",
    ]);

    assert_eq!(github_default_branch(&repo, "").as_deref(), Some("trunk"));

    std::fs::remove_dir_all(repo).expect("temporary repo should be removable");
}

#[test]
fn regular_creation_leaves_the_primary_worktree_directory_unchanged() {
    let worktrees = vec![
        (std::path::PathBuf::from("/repo"), Some("trunk".into())),
        (
            std::path::PathBuf::from("/worktrees/feature"),
            Some("feature".into()),
        ),
    ];
    assert_eq!(
        regular_creation_directory_from_worktrees(
            std::path::Path::new("/repo/crates/app"),
            std::path::Path::new("/repo"),
            &worktrees,
            Some("trunk"),
        ),
        std::path::PathBuf::from("/repo/crates/app")
    );
}

#[test]
fn regular_creation_leaves_a_linked_worktree_for_the_github_default() {
    let worktrees = vec![
        (
            std::path::PathBuf::from("/repo"),
            Some("maintenance".into()),
        ),
        (
            std::path::PathBuf::from("/worktrees/feature"),
            Some("feature".into()),
        ),
        (
            std::path::PathBuf::from("/worktrees/trunk"),
            Some("trunk".into()),
        ),
    ];
    assert_eq!(
        regular_creation_directory_from_worktrees(
            std::path::Path::new("/worktrees/feature/crates/app"),
            std::path::Path::new("/worktrees/feature"),
            &worktrees,
            Some("trunk"),
        ),
        std::path::PathBuf::from("/worktrees/trunk")
    );
}

#[test]
fn regular_creation_resolves_a_real_linked_checkout_to_the_primary_repo() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let scratch = std::env::temp_dir().join(format!(
        "muxtrix-regular-pane-worktree-test-{}-{unique}",
        std::process::id()
    ));
    let repo = scratch.join("repo");
    let worktree = scratch.join("feature");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = |cwd: &std::path::Path, args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "muxtrix@example.test"]);
    git(&repo, &["config", "user.name", "Muxtrix test"]);
    std::fs::write(repo.join("README.md"), "fixture\n").expect("fixture file");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "fixture"]);
    let worktree_arg = worktree.to_string_lossy().into_owned();
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feature", &worktree_arg],
    );
    let primary_nested = repo.join("crates/app");
    let linked_nested = worktree.join("crates/app");
    std::fs::create_dir_all(&primary_nested).expect("primary nested directory");
    std::fs::create_dir_all(&linked_nested).expect("linked nested directory");

    assert_eq!(
        resolve_regular_creation_directory(&primary_nested, ""),
        primary_nested,
        "a primary-checkout pane should preserve its exact cwd"
    );
    assert_eq!(
        resolve_regular_creation_directory(&linked_nested, ""),
        repo,
        "a linked-worktree pane should launch at the main checkout root"
    );

    std::fs::remove_dir_all(scratch).expect("temporary repo should be removable");
}

#[test]
fn worktree_defaults_fall_back_to_main_then_master_then_primary() {
    let main = vec![
        (std::path::PathBuf::from("/repo"), Some("release".into())),
        (
            std::path::PathBuf::from("/worktrees/main"),
            Some("main".into()),
        ),
        (
            std::path::PathBuf::from("/worktrees/master"),
            Some("master".into()),
        ),
    ];
    assert_eq!(
        preferred_default_worktree(&main, None),
        std::path::PathBuf::from("/worktrees/main")
    );
    assert_eq!(
        preferred_default_worktree(&main[..1], None),
        std::path::PathBuf::from("/repo")
    );
}

#[test]
fn only_the_primary_worktree_is_protected_from_deletion() {
    assert_eq!(
        worktree_deletion_blocker(true).as_deref(),
        Some("Primary worktree")
    );
    assert!(worktree_deletion_blocker(false).is_none());
}

#[test]
fn manager_rejects_protected_worktree_deletion_at_the_action_boundary() {
    let mut app = Muxtrix::new();
    app.worktree_manager = Some(WorktreeManagerState {
        mode: WorktreeManagerMode::Manage,
        generation: 1,
        repo_root: Some("/repo".into()),
        failure: None,
        entries: vec![WorktreeManagerEntry {
            path: "/repo".into(),
            branch: Some("trunk".into()),
            unpushed_commits: 0,
            deletion_blocker: Some("Primary worktree".into()),
            used_by: None,
        }],
        loading: false,
        selected: 0,
        busy: false,
        error: None,
        restart_target: None,
    });

    let _ = app.delete_worktree_entry(0);

    assert_eq!(
        app.worktree_manager
            .as_ref()
            .and_then(|manager| manager.error.as_deref()),
        Some("repo is the Primary worktree and cannot be deleted")
    );
}

#[test]
fn manage_worktrees_opens_settings_before_repository_discovery_finishes() {
    let mut app = Muxtrix::new();

    let _discovery = app.open_worktree_manager();

    assert_eq!(app.active_view, ActiveView::Settings);
    assert_eq!(app.settings_page, SettingsPage::Worktrees);
    let manager = app
        .worktree_manager
        .as_ref()
        .expect("manager should paint a loading state immediately");
    assert!(manager.loading);
    assert!(manager.entries.is_empty());
}

fn worktree_settings_app_with_entries() -> Muxtrix {
    let mut app = Muxtrix::new();
    app.active_view = ActiveView::Settings;
    app.settings_page = SettingsPage::Worktrees;
    app.worktree_manager = Some(WorktreeManagerState {
        mode: WorktreeManagerMode::Manage,
        generation: 1,
        repo_root: Some("/repo".into()),
        failure: None,
        entries: vec![
            WorktreeManagerEntry {
                path: "/repo".into(),
                branch: Some("trunk".into()),
                unpushed_commits: 0,
                deletion_blocker: Some("Primary worktree".into()),
                used_by: None,
            },
            WorktreeManagerEntry {
                path: "/repo/checkouts/feature".into(),
                branch: Some("feature".into()),
                unpushed_commits: 0,
                deletion_blocker: None,
                used_by: None,
            },
        ],
        loading: false,
        selected: 0,
        busy: false,
        error: None,
        restart_target: None,
    });
    app
}

/// The page prints `↑↓ Select` in its footer, so the arrows have to reach
/// the inventory. The generic non-workspace branch used to consume every
/// key first, leaving the advertised navigation dead on arrival.
#[test]
fn worktree_settings_arrows_move_the_selection() {
    let mut app = worktree_settings_app_with_entries();

    let _ = app.handle_keyboard(key_press(
        Key::Named(Named::ArrowDown),
        Modifiers::default(),
    ));

    assert_eq!(
        app.worktree_manager
            .as_ref()
            .map(|manager| manager.selected),
        Some(1)
    );

    let _ = app.handle_keyboard(key_press(Key::Named(Named::ArrowUp), Modifiers::default()));

    assert_eq!(
        app.worktree_manager
            .as_ref()
            .map(|manager| manager.selected),
        Some(0)
    );
}

/// Manage renders only as the settings page, never as a dismissible
/// dialog, so Enter has nothing to confirm — and must not throw the
/// inventory away and strand the page on its "not loaded" notice.
#[test]
fn worktree_settings_enter_keeps_the_inventory() {
    let mut app = worktree_settings_app_with_entries();

    let _ = app.handle_keyboard(key_press(Key::Named(Named::Enter), Modifiers::default()));

    assert!(app.worktree_manager.is_some());
    assert_eq!(app.active_view, ActiveView::Settings);
}

/// `Del` is advertised in the footer, so it has to reach the removal
/// boundary. The protected row is the cheapest proof of routing: it fails
/// inside `delete_worktree_entry` without shelling out to Git.
#[test]
fn worktree_settings_delete_reaches_the_removal_boundary() {
    let mut app = worktree_settings_app_with_entries();

    let _ = app.handle_keyboard(key_press(Key::Named(Named::Delete), Modifiers::default()));

    assert_eq!(
        app.worktree_manager
            .as_ref()
            .and_then(|manager| manager.error.as_deref()),
        Some("repo is the Primary worktree and cannot be deleted")
    );
}

/// Backspace reads as "go back" on a full-window page and the footer only
/// advertises `Del`, so it must not quietly remove the selected checkout.
#[test]
fn worktree_settings_backspace_does_not_remove_a_checkout() {
    let mut app = worktree_settings_app_with_entries();
    app.worktree_manager
        .as_mut()
        .expect("staged manager")
        .selected = 1;

    let _ = app.handle_keyboard(key_press(
        Key::Named(Named::Backspace),
        Modifiers::default(),
    ));

    let manager = app.worktree_manager.as_ref().expect("staged manager");
    assert!(manager.error.is_none());
    assert!(!manager.busy);
    assert_eq!(manager.entries.len(), 2);
}

/// Escape still returns to the terminal, and discards the settings draft
/// on the way out exactly as it does from every other settings page.
#[test]
fn worktree_settings_escape_returns_to_the_terminal_and_discards_the_draft() {
    let mut app = worktree_settings_app_with_entries();
    app.settings_draft.ui_font_size = app.settings.ui_font_size + 2.0;

    let _ = app.handle_keyboard(key_press(Key::Named(Named::Escape), Modifiers::default()));

    assert_eq!(app.active_view, ActiveView::Workspace);
    assert_eq!(app.settings_draft.ui_font_size, app.settings.ui_font_size);
}

/// The settings top bar's crowding is judged in label widths, not pixels:
/// the same window that comfortably holds the long return label at the
/// default interface type size cannot hold it once the type is scaled up.
#[test]
fn settings_nav_crowding_follows_the_interface_type_size() {
    // The narrowest supported window keeps the sentence at the sizes the
    // bar was drawn for.
    let mut settings = AppSettings::default();
    assert!(!settings_nav_is_crowded(720.0, &settings));

    // Scaled-up interface type crowds that same window, so the label
    // shortens rather than closing the gap to the page switch.
    settings.ui_font_size = 20.0;
    assert!(settings_nav_is_crowded(720.0, &settings));

    // Width still buys the sentence back at that type size.
    assert!(!settings_nav_is_crowded(1440.0, &settings));

    // The threshold moves with the type size rather than sitting on a
    // fixed pixel width.
    settings.ui_font_size = 12.0;
    let small = settings.ui_pixels(SETTINGS_NAV_LABEL_POINTS) * SETTINGS_NAV_LABEL_WIDTHS;
    settings.ui_font_size = 20.0;
    let large = settings.ui_pixels(SETTINGS_NAV_LABEL_POINTS) * SETTINGS_NAV_LABEL_WIDTHS;
    assert!(large > small);
}

/// Lanes are derived once and shared by the header, every row, and the
/// ellipsis budgets, so a wider window widens the copy lanes instead of
/// wrapping a long branch inside a fixed box.
#[test]
fn worktree_lanes_spend_extra_width_on_the_copy_lanes() {
    let narrow = WorktreeLanes::for_window(1000.0, false);
    let wide = WorktreeLanes::for_window(1400.0, false);

    assert!(wide.identity > narrow.identity);
    assert!(wide.branch > narrow.branch);
    assert_eq!(wide.status, narrow.status);
    assert_eq!(wide.commits, narrow.commits);
    assert_eq!(wide.action, narrow.action);

    // Past the cap the table stops growing rather than stranding the
    // action lane a screen away from the row it acts on.
    let capped = WorktreeLanes::for_window(WORKTREE_PAGE_MAX_WIDTH + 400.0, false);
    let beyond = WorktreeLanes::for_window(WORKTREE_PAGE_MAX_WIDTH + 900.0, false);
    assert_eq!(capped.identity, beyond.identity);

    // Stacked rows give identity the whole line rather than the narrow
    // slice the table layout would leave it.
    let stacked = WorktreeLanes::for_window(820.0, true);
    let tabular = WorktreeLanes::for_window(820.0, false);
    assert!(stacked.identity > tabular.identity);
}

#[test]
fn worktree_discovery_ignores_results_from_an_older_request() {
    let mut app = Muxtrix::new();
    app.worktree_manager = Some(WorktreeManagerState {
        mode: WorktreeManagerMode::Manage,
        generation: 2,
        repo_root: None,
        failure: None,
        entries: Vec::new(),
        loading: true,
        selected: 0,
        busy: false,
        error: None,
        restart_target: None,
    });

    let _ = app.update(Message::WorktreeManagerLoaded(
        1,
        Ok(WorktreeManagerDiscovery {
            repo_root: Some("/stale".into()),
            failure: None,
            entries: Vec::new(),
        }),
    ));

    let manager = app.worktree_manager.as_ref().expect("manager remains open");
    assert!(manager.loading);
    assert!(manager.repo_root.is_none());
}

#[test]
fn remove_unused_targets_only_unprotected_worktrees_without_panes() {
    let entry = |name: &str, blocker: Option<&str>, used_by: Option<&str>| WorktreeManagerEntry {
        path: std::path::PathBuf::from("/worktrees").join(name),
        branch: Some(name.into()),
        unpushed_commits: 0,
        deletion_blocker: blocker.map(str::to_owned),
        used_by: used_by.map(str::to_owned),
    };
    let entries = vec![
        entry("main", Some("Primary worktree"), None),
        entry("active", None, Some("Agent pane")),
        entry("unused-one", None, None),
        entry("unused-two", None, None),
    ];

    assert_eq!(
        unused_worktree_paths(&entries),
        vec![
            std::path::PathBuf::from("/worktrees/unused-one"),
            std::path::PathBuf::from("/worktrees/unused-two"),
        ]
    );
}

#[test]
fn removing_linked_default_branch_worktree_preserves_its_branch() {
    let scratch = std::env::temp_dir().join(format!(
        "muxtrix-bulk-worktree-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    let repo = scratch.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("file"), "one").expect("file");
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    git(&["branch", "-m", "release"]);
    let default = scratch.join("worktrees").join("main");
    let feature = scratch.join("worktrees").join("feature");
    create_git_worktree(&repo, &default, "main", "").expect("default worktree");
    create_git_worktree(&repo, &feature, "feature", "").expect("feature worktree");

    let (removed, result) = remove_git_worktrees(&repo, vec![default.clone(), feature.clone()], "");

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(removed, vec![default.clone(), feature.clone()]);
    assert!(!default.exists());
    assert!(!feature.exists());
    assert!(repo.exists());
    git(&["show-ref", "--verify", "refs/heads/main"]);
    git(&["show-ref", "--verify", "refs/heads/feature"]);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn unpushed_status_counts_commits_missing_from_every_remote_ref() {
    let scratch = std::env::temp_dir().join(format!(
        "muxtrix-unpushed-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("repo dir");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&scratch)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(scratch.join("file"), "one").expect("first file");
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    assert_eq!(unpushed_commit_count(&scratch, ""), 0);

    std::fs::write(scratch.join("file"), "two").expect("second file");
    git(&["commit", "-qam", "second"]);
    assert_eq!(unpushed_commit_count(&scratch, ""), 1);

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn worktree_creation_survives_stale_branches_and_registrations() {
    let scratch = std::env::temp_dir().join(format!(
        "muxtrix-worktree-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    let repo = scratch.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = |args: &[&str], cwd: &std::path::Path| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"], &repo);
    std::fs::write(repo.join("file"), "x").expect("file");
    git(&["add", "."], &repo);
    git(&["commit", "-qm", "init"], &repo);

    let first = scratch.join("worktrees").join("worktree-1");
    create_git_worktree(&repo, &first, "worktree-1", "").expect("first worktree should create");
    assert!(first.join(".git").exists());

    // Simulate the breakage: the user deletes the folder by hand, leaving
    // a stale registration AND the branch behind.
    std::fs::remove_dir_all(&first).expect("delete worktree dir");
    let retry = create_git_worktree(&repo, &first, "worktree-1", "");
    assert!(
        retry.is_ok(),
        "stale registration + existing branch must not break creation: {retry:?}"
    );
    assert!(first.join(".git").exists());

    // A branch that exists WITH a live worktree still errors clearly.
    let second = scratch.join("worktrees").join("worktree-1-copy");
    let conflict = create_git_worktree(&repo, &second, "worktree-1", "");
    assert!(conflict.is_err());
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn worktree_commands_require_a_git_repository() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let non_repository =
        std::env::temp_dir().join(format!("muxtrix-non-repository-{}", uuid::Uuid::new_v4()));
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Idle,
            activity: None,
            session_id: None,
            cwd: Some(non_repository.display().to_string()),
            git_branch: None,
        },
    );
    // The agent-reported cwd wins over the live shell process, so pointing
    // it at a deliberately missing path guarantees a non-repository
    // context even when a test harness places a .git marker in /tmp. The
    // dialog still opens with the explanation, but creation is impossible.
    let _ = app.run_command(CommandAction::NewWorktree(commands::WorktreeKind::Pane(
        SplitAxis::Horizontal,
    )));
    let prompt = app
        .worktree_prompt
        .as_ref()
        .expect("the dialog should open to explain the failure");
    assert!(prompt.repo_root.is_none());
    let _ = app.update(Message::ConfirmWorktree);
    assert!(
        app.worktree_prompt.is_some(),
        "confirm must be inert without a repository"
    );
}

#[test]
fn worktree_agent_commands_require_an_installed_default_agent() {
    let mut app = Muxtrix::new();

    let _ = app.run_command(CommandAction::NewWorktreeWithAgent(
        commands::WorktreeKind::Pane(SplitAxis::Horizontal),
    ));
    assert!(app.default_agent_prompt);
    assert!(app.worktree_prompt.is_none());

    app.default_agent_prompt = false;
    app.settings.default_agent = Some(Agent::Codex);
    let _ = app.run_command(CommandAction::RestartPaneInWorktreeWithAgent);
    assert!(
        app.default_agent_prompt,
        "a saved choice whose hooks are not installed must remain gated"
    );

    app.default_agent_prompt = false;
    app.hook_statuses.push(HookStatus {
        agent: Agent::Codex,
        scope: HookScope::User,
        target: "/tmp/codex-hooks.json".into(),
        installed: true,
        managed_entries: 8,
        backup_available: false,
        unreachable_entries: 0,
    });
    let pane_id = active_pane_id(&app);
    let _ = app.run_command(CommandAction::RestartPaneInWorktreeWithAgent);
    assert_eq!(
        app.worktree_prompt
            .as_ref()
            .expect("configured command should open worktree prompt")
            .target,
        WorktreePromptTarget::RestartPaneWithAgent(pane_id, Agent::Codex)
    );
    assert!(!app.default_agent_prompt);
}

#[test]
fn configured_default_agent_resumes_the_pending_worktree_command() {
    let mut app = Muxtrix::new();
    let action =
        CommandAction::NewWorktreeWithAgent(commands::WorktreeKind::Pane(SplitAxis::Horizontal));

    let _ = app.run_command(action);
    assert_eq!(app.pending_default_agent_command, Some(action));

    let _ = app.update(Message::OpenDefaultAgentSettings);
    assert_eq!(app.pending_default_agent_command, Some(action));
    assert_eq!(app.active_view, ActiveView::Settings);

    app.settings.default_agent = Some(Agent::Codex);
    app.settings_draft.default_agent = Some(Agent::Codex);
    app.hook_statuses.push(HookStatus {
        agent: Agent::Codex,
        scope: HookScope::User,
        target: "/tmp/codex-hooks.json".into(),
        installed: true,
        managed_entries: 8,
        backup_available: false,
        unreachable_entries: 0,
    });

    let _ = app.resume_pending_default_agent_command();
    assert!(app.pending_default_agent_command.is_none());
    assert_eq!(app.active_view, ActiveView::Workspace);
    assert!(matches!(
        app.worktree_prompt.as_ref().map(|prompt| prompt.target),
        Some(WorktreePromptTarget::OpenWithAgent(
            commands::WorktreeKind::Pane(SplitAxis::Horizontal),
            Agent::Codex
        ))
    ));
}

#[test]
fn dismissing_default_agent_setup_cancels_the_pending_command() {
    let mut app = Muxtrix::new();
    let action = CommandAction::RestartPaneInExistingWorktreeWithAgent;

    let _ = app.run_command(action);
    assert_eq!(app.pending_default_agent_command, Some(action));
    let _ = app.update(Message::CloseDefaultAgentPrompt);

    assert!(!app.default_agent_prompt);
    assert!(app.pending_default_agent_command.is_none());
    assert!(app.worktree_manager.is_none());
}

#[test]
fn blank_agent_command_cannot_pass_the_configuration_gate_or_open_a_pane() {
    let mut app = Muxtrix::new();
    app.settings.default_agent = Some(Agent::Codex);
    app.settings.codex_command = "   ".into();
    app.hook_statuses.push(HookStatus {
        agent: Agent::Codex,
        scope: HookScope::User,
        target: "/tmp/codex-hooks.json".into(),
        installed: true,
        managed_entries: 8,
        backup_available: false,
        unreachable_entries: 0,
    });
    let terminal_count = app.terminals.len();

    let _ = app.run_command(CommandAction::RestartPaneInWorktreeWithAgent);
    assert!(app.default_agent_prompt);
    assert!(app.worktree_prompt.is_none());

    let error = app
        .launch_agent(Agent::Codex)
        .expect_err("a blank launch command must fail before splitting");
    assert!(error.contains("Set a launch command"));
    assert_eq!(app.terminals.len(), terminal_count);
}

#[test]
fn terminal_launch_failure_marks_a_queued_agent_as_failed() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.start_agent_in_pane(Agent::Codex, pane_id)
        .expect("agent start should queue for the initial terminal");

    app.mark_terminal_launch_failed(pane_id, "host unavailable".into());

    assert!(!app.pending_terminal_input.contains_key(&pane_id));
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Failed);
    assert_eq!(
        app.agent_statuses[&pane_id].activity.as_deref(),
        Some("Terminal failed before the agent could start")
    );
}

#[test]
fn created_worktree_with_agent_starts_the_agent_in_the_new_pane() {
    let mut app = Muxtrix::new();
    app.settings.codex_command = "true".into();
    let previous = active_pane_id(&app);

    app.open_created_worktree(
        WorktreePromptTarget::OpenWithAgent(
            commands::WorktreeKind::Pane(SplitAxis::Vertical),
            Agent::Codex,
        ),
        std::env::temp_dir(),
    )
    .expect("worktree pane and agent should launch");

    let pane_id = active_pane_id(&app);
    assert_ne!(pane_id, previous);
    assert_eq!(app.agent_statuses[&pane_id].agent, "codex");
    assert!(matches!(
        app.agent_statuses[&pane_id].state,
        AgentState::Idle | AgentState::Running
    ));
    assert!(matches!(
        active_tab(&app).root,
        PaneTree::Split {
            axis: SplitAxis::Vertical,
            ..
        }
    ));
}

#[test]
fn existing_worktree_agent_command_keeps_the_current_pane_target() {
    let mut app = Muxtrix::new();
    app.settings.default_agent = Some(Agent::Claude);
    app.hook_statuses.push(HookStatus {
        agent: Agent::Claude,
        scope: HookScope::User,
        target: "/tmp/claude-settings.json".into(),
        installed: true,
        managed_entries: 9,
        backup_available: false,
        unreachable_entries: 0,
    });
    let pane_id = active_pane_id(&app);

    let _ = app.run_command(CommandAction::RestartPaneInExistingWorktreeWithAgent);

    assert_eq!(
        app.worktree_manager
            .as_ref()
            .expect("existing worktree picker should open")
            .mode,
        WorktreeManagerMode::RestartPaneWithAgent(pane_id, Agent::Claude)
    );
}

#[test]
fn worktree_restart_commands_create_or_reuse_without_stale_agent_cwd() {
    let scratch = std::env::temp_dir().join(format!(
        "muxtrix-worktree-switcher-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    let repo = scratch.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = |args: &[&str], cwd: &std::path::Path| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"], &repo);
    std::fs::write(repo.join("file"), "x").expect("file");
    git(&["add", "."], &repo);
    git(&["commit", "-qm", "init"], &repo);
    let alternate = scratch.join("alternate");
    create_git_worktree(&repo, &alternate, "alternate", "")
        .expect("alternate worktree should be created");

    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    if let Some(runtime) = app.terminals.remove(&pane_id)
        && let Some(session) = &runtime.session
    {
        session.terminate();
    }
    let pane = app
        .session
        .workspaces
        .iter_mut()
        .find_map(|workspace| workspace.pane_mut(pane_id))
        .expect("pane should exist");
    let surface = pane
        .surfaces
        .iter_mut()
        .find(|surface| surface.id == pane.active_surface_id)
        .expect("active surface should exist");
    let muxtrix_domain::SurfaceKind::Terminal(terminal) = &mut surface.kind else {
        panic!("surface should be terminal");
    };
    terminal.working_directory = Some(repo.clone());
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "codex".into(),
            display_name: None,
            state: AgentState::Running,
            activity: None,
            session_id: Some("stale".into()),
            cwd: Some("/definitely/not/the/repository".into()),
            git_branch: None,
        },
    );

    let _ = app.run_command(CommandAction::RestartPaneInWorktree);
    let prompt = app
        .worktree_prompt
        .as_ref()
        .expect("restart should open the worktree creation prompt");
    assert_eq!(prompt.target, WorktreePromptTarget::RestartPane(pane_id));
    assert_eq!(prompt.repo_root.as_deref(), Some(repo.as_path()));
    assert!(app.worktree_manager.is_none());

    app.worktree_prompt = None;
    let _ = app.run_command(CommandAction::RestartPaneInExistingWorktree);
    let manager = app
        .worktree_manager
        .as_ref()
        .expect("existing-worktree command should open the picker");
    assert_eq!(manager.mode, WorktreeManagerMode::RestartPane(pane_id));
    assert!(manager.loading);
    let generation = manager.generation;
    let discovery = discover_worktree_manager(
        WorktreeManagerMode::RestartPane(pane_id),
        Some(repo.clone()),
        "",
    )
    .expect("worktree discovery should succeed");
    let _ = app.update(Message::WorktreeManagerLoaded(generation, Ok(discovery)));
    let manager = app
        .worktree_manager
        .as_ref()
        .expect("existing-worktree command should keep the picker open");
    assert_eq!(manager.entries.len(), 1);
    assert_eq!(manager.entries[0].path, alternate);
    assert!(manager.failure.is_none());

    let _ = app.update(Message::WorktreeManagerRestart(0));
    assert_eq!(
        app.worktree_manager
            .as_ref()
            .expect("switcher should remain open for confirmation")
            .restart_target,
        Some(0)
    );
    let _ = app.update(Message::CancelWorktreeManagerRestart);
    assert_eq!(
        app.worktree_manager
            .as_ref()
            .expect("switcher should remain open after going back")
            .restart_target,
        None
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn created_worktree_restart_preserves_identity_and_clears_transient_state() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let target = std::env::temp_dir();
    {
        let pane = app
            .active_workspace_mut()
            .expect("workspace should exist")
            .pane_mut(pane_id)
            .expect("pane should exist");
        pane.custom_name = Some("kept name".into());
        pane.attention.unread_count = 2;
        pane.attention.message = Some("stale attention".into());
    }
    app.notifications.push(AgentNotification {
        pane_id,
        unread: true,
    });
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Running,
            activity: Some("stale activity".into()),
            session_id: Some("stale-session".into()),
            cwd: Some("/old/worktree".into()),
            git_branch: Some("old".into()),
        },
    );
    app.detected_agents
        .insert(pane_id, std::time::Instant::now());

    app.open_created_worktree(WorktreePromptTarget::RestartPane(pane_id), target.clone())
        .expect("fresh terminal should launch in the requested directory");

    assert_eq!(active_pane_id(&app), pane_id);
    let pane = app
        .active_workspace()
        .expect("workspace should exist")
        .pane(pane_id)
        .expect("pane identity should survive");
    assert_eq!(pane.custom_name.as_deref(), Some("kept name"));
    assert_eq!(pane.attention.unread_count, 0);
    assert!(pane.attention.message.is_none());
    let terminal = pane
        .active_surface()
        .and_then(|surface| match &surface.kind {
            muxtrix_domain::SurfaceKind::Terminal(terminal) => Some(terminal),
            _ => None,
        });
    assert_eq!(
        terminal.and_then(|terminal| terminal.working_directory.as_ref()),
        Some(&target)
    );
    assert!(app.terminals[&pane_id].session.is_some());
    assert!(!app.agent_statuses.contains_key(&pane_id));
    assert!(!app.detected_agents.contains_key(&pane_id));
    assert!(
        app.notifications
            .iter()
            .all(|notification| notification.pane_id != pane_id)
    );
}

#[test]
fn worktree_panes_apply_the_requested_split_axis() {
    for expected_axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
        let mut app = Muxtrix::new();
        app.open_worktree(
            commands::WorktreeKind::Pane(expected_axis),
            std::env::temp_dir(),
        )
        .expect("worktree pane should open");

        let root = &active_tab(&app).root;
        assert!(
            matches!(root, PaneTree::Split { axis, .. } if *axis == expected_axis),
            "worktree pane should use the requested split axis: {root:?}"
        );
    }
}

#[test]
fn pane_local_agent_commands_set_identity_before_the_first_hook() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.observe_terminal_command(pane_id, b"codex --resume\r");
    assert_eq!(app.agent_statuses[&pane_id].agent, "codex");
    assert_eq!(app.pane_state_label(pane_id), "Running");
    assert_eq!(app.pane_signal_kind(pane_id, false), PaneSignalKind::Active);

    assert_eq!(
        agent_command("/home/user/bin/claude --continue", &app.settings),
        Some(Agent::Claude)
    );
    assert_eq!(agent_command("omp", &app.settings), Some(Agent::Pi));
    assert_eq!(agent_command("cargo test", &app.settings), None);
}

#[test]
fn pi_session_start_cannot_relabel_a_launched_agent_idle() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let pane = Some(pane_id.as_uuid().to_string());

    app.observe_terminal_command(pane_id, b"omp\r");
    assert_eq!(app.agent_statuses[&pane_id].agent, "pi");
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);

    let start = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "pi".into(),
        state: AgentState::Idle,
        event: Some("session_start".into()),
        title: "Oh My Pi".into(),
        body: "Ready for input".into(),
        pane_id: pane,
        session_id: Some("pi-session-1".into()),
        cwd: None,
    });

    assert!(start.ok);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);
    assert_eq!(app.pane_state_label(pane_id), "Running");
}

#[test]
fn pane_dimensions_map_to_terminal_rows_and_columns() {
    let settings = AppSettings::default();
    assert_eq!(
        pty_size_for_pane(Size::new(856.0, 400.0), &settings),
        if cfg!(target_os = "macos") {
            PtySize {
                rows: 23,
                cols: 100,
                pixel_width: 840,
                pixel_height: 384,
            }
        } else {
            PtySize {
                rows: 17,
                cols: 75,
                pixel_width: 840,
                pixel_height: 384,
            }
        }
    );
    let minimum = pty_size_for_pane(Size::new(0.0, 0.0), &settings);
    assert_eq!((minimum.cols, minimum.rows), (2, 2));

    let one_pixel_wider = pty_size_for_pane(Size::new(857.0, 401.0), &settings);
    let baseline = pty_size_for_pane(Size::new(856.0, 400.0), &settings);
    assert!(!terminal_grid_changed(baseline, one_pixel_wider));
}

#[test]
fn stale_terminal_frames_are_rejected_after_a_grid_resize() {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal actor should start");
    let snapshot = actor.snapshot().expect("snapshot should render");
    assert!(snapshot_matches_grid(&snapshot, initial_pty_size()));
    assert!(!snapshot_matches_grid(
        &snapshot,
        PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 1_000,
            pixel_height: 600,
        }
    ));
    actor.shutdown().expect("terminal actor should stop");
}

#[cfg(unix)]
#[test]
fn grid_resize_keeps_the_last_frame_visible_until_the_new_grid_arrives() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        if app.terminals[&pane_id].snapshot.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let previous = app.terminals[&pane_id]
        .snapshot
        .clone()
        .expect("the initial terminal frame should arrive");

    app.terminals
        .get_mut(&pane_id)
        .expect("runtime should exist")
        .resize(Size::new(800.0, 500.0), &app.settings)
        .expect("terminal resize should queue");
    let runtime = &app.terminals[&pane_id];
    assert_eq!(runtime.snapshot.as_ref(), Some(&previous));
    assert!(!snapshot_matches_grid(&previous, runtime.size));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        app.poll_terminal();
        if app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot_matches_grid(snapshot, app.terminals[&pane_id].size))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        app.terminals[&pane_id]
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot_matches_grid(snapshot, app.terminals[&pane_id].size)),
        "the resized terminal frame should replace the retained frame"
    );
}

#[test]
fn fleet_context_combines_a_real_git_branch_and_directory() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "muxtrix-git-context-{}-{unique}",
        std::process::id()
    ));
    let nested = root.join("src").join("nested");
    std::fs::create_dir_all(root.join(".git")).expect("git metadata directory should exist");
    std::fs::create_dir_all(&nested).expect("nested working directory should exist");
    std::fs::write(
        root.join(".git").join("HEAD"),
        "ref: refs/heads/feature/fleet\n",
    )
    .expect("HEAD should be writable");

    assert_eq!(
        git_branch_for_directory(nested.to_str()),
        Some("feature/fleet".into())
    );
    std::fs::remove_dir_all(root).expect("temporary git metadata should be removable");
}

#[test]
fn pane_signal_semantics_distinguish_activity_attention_and_failure() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);

    assert_eq!(
        app.pane_signal_kind(pane_id, false),
        PaneSignalKind::Neutral
    );
    for (state, expected) in [
        (AgentState::Idle, PaneSignalKind::Subtle),
        (AgentState::Running, PaneSignalKind::Active),
        (AgentState::Waiting, PaneSignalKind::Warning),
        (AgentState::Completed, PaneSignalKind::Neutral),
        (AgentState::Failed, PaneSignalKind::Danger),
        (AgentState::Stopped, PaneSignalKind::Subtle),
    ] {
        app.agent_statuses.insert(
            pane_id,
            AgentPaneStatus {
                agent: "codex".into(),
                display_name: None,
                state,
                activity: None,
                session_id: None,
                cwd: None,
                git_branch: None,
            },
        );
        assert_eq!(app.pane_signal_kind(pane_id, true), expected);
    }

    app.agent_statuses.remove(&pane_id);
    assert_eq!(app.pane_signal_kind(pane_id, true), PaneSignalKind::Warning);
}

#[test]
fn pending_wsl_launch_is_neutral_and_truthfully_rolls_up_as_starting() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);

    app.terminals
        .insert(pane_id, TerminalRuntime::preparing_host("WSL shell"));
    assert_eq!(app.pane_state_label(pane_id), "Preparing");
    assert_eq!(
        app.pane_signal_kind(pane_id, false),
        PaneSignalKind::Neutral
    );
    assert_eq!(
        app.workspace_state_label(app.active_workspace().expect("active workspace")),
        "Starting"
    );

    app.terminals
        .insert(pane_id, TerminalRuntime::starting("WSL shell", 1, None));
    assert_eq!(app.pane_state_label(pane_id), "Starting");
    assert_eq!(
        app.pane_signal_kind(pane_id, false),
        PaneSignalKind::Neutral
    );
    assert_eq!(
        app.workspace_state_label(app.active_workspace().expect("active workspace")),
        "Starting"
    );

    assert_eq!(
        app.pane_signal_kind(pane_id, true),
        PaneSignalKind::Warning,
        "real unread attention must still outrank launch progress"
    );
}

#[test]
fn foreign_agent_events_cannot_demote_a_live_agent_pane() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let pane = Some(pane_id.as_uuid().to_string());
    let running = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("UserPromptSubmit".into()),
        title: "Codex · UserPromptSubmit".into(),
        body: "Working".into(),
        pane_id: pane.clone(),
        session_id: Some("codex-1".into()),
        cwd: None,
    });
    assert!(running.ok);
    // A claude lifecycle event arriving with this pane's id (a stray
    // inherited MUXTRIX_PANE_ID) must not stop or overwrite codex.
    let foreign = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "claude".into(),
        state: AgentState::Stopped,
        event: Some("SessionEnd".into()),
        title: "Claude · SessionEnd".into(),
        body: "bye".into(),
        pane_id: pane,
        session_id: Some("claude-9".into()),
        cwd: None,
    });
    assert!(foreign.ok);
    let status = &app.agent_statuses[&pane_id];
    assert_eq!(status.agent, "codex");
    assert_eq!(status.state, AgentState::Running);
}

#[test]
fn delayed_session_start_cannot_regress_the_first_running_prompt() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let pane = Some(pane_id.as_uuid().to_string());

    let running = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("UserPromptSubmit".into()),
        title: "Codex · UserPromptSubmit".into(),
        body: "Implement the feature".into(),
        pane_id: pane.clone(),
        session_id: Some("thread-1".into()),
        cwd: None,
    });
    assert!(running.ok);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);
    assert_eq!(app.pane_signal_kind(pane_id, false), PaneSignalKind::Active);

    let delayed_start = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Idle,
        event: Some("SessionStart".into()),
        title: "Codex · SessionStart".into(),
        body: "Ready for input".into(),
        pane_id: pane.clone(),
        session_id: Some("thread-1".into()),
        cwd: None,
    });
    assert!(delayed_start.ok);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);
    assert_eq!(
        app.agent_statuses[&pane_id].activity.as_deref(),
        Some("Implement the feature")
    );
    assert_eq!(app.pane_signal_kind(pane_id, false), PaneSignalKind::Active);

    let next_session = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Idle,
        event: Some("SessionStart".into()),
        title: "Codex · SessionStart".into(),
        body: "Ready for input".into(),
        pane_id: pane,
        session_id: Some("thread-2".into()),
        cwd: None,
    });
    assert!(next_session.ok);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Idle);
    assert_eq!(app.pane_signal_kind(pane_id, false), PaneSignalKind::Subtle);
}

#[test]
fn ctrl_c_marks_only_the_interrupted_running_agent_idle() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Running,
            activity: Some("Working".into()),
            session_id: Some("session-1".into()),
            cwd: None,
            git_branch: None,
        },
    );

    app.observe_agent_interrupt(pane_id, b"ordinary input");
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);

    app.observe_agent_interrupt(pane_id, &[0x03]);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Idle);
    assert_eq!(
        app.agent_statuses[&pane_id].activity.as_deref(),
        Some("Prompt interrupted")
    );
    assert_eq!(app.pane_signal_kind(pane_id, false), PaneSignalKind::Subtle);
}

#[test]
fn terminal_wheel_delta_maps_to_ghostty_scrollback_lines() {
    assert_eq!(
        terminal_scroll_lines(ScrollDelta::Lines { x: 0.0, y: 1.0 }, 20.0),
        -3
    );
    assert_eq!(
        terminal_scroll_lines(ScrollDelta::Pixels { x: 0.0, y: -25.0 }, 10.0),
        3
    );
    assert_eq!(
        terminal_scroll_lines(ScrollDelta::Pixels { x: 0.0, y: 0.0 }, 10.0),
        0
    );
}

#[test]
fn split_drag_updates_and_clamps_the_target_ratio() {
    let mut app = Muxtrix::new();
    app.split_terminal(SplitAxis::Horizontal)
        .expect("horizontal split should succeed");
    let workspace_id = app.active_workspace().expect("workspace should exist").id;
    let tab_id = app
        .active_workspace()
        .expect("workspace should exist")
        .active_tab_id;
    let key = SplitKey {
        workspace_id,
        tab_id,
        path: Vec::new(),
    };
    app.split_sizes
        .insert(key.clone(), Size::new(1_000.0, 600.0));
    app.cursor_position = Point::new(500.0, 300.0);
    app.begin_split_drag(key.clone(), SplitAxis::Horizontal)
        .expect("split drag should begin");
    app.update_split_drag(Point::new(700.0, 300.0))
        .expect("split drag should update");
    assert_eq!(
        split_ratio_at(&active_tab(&app).root, &[])
            .expect("root should be split")
            .permille(),
        700
    );
    app.update_split_drag(Point::new(2_000.0, 300.0))
        .expect("split drag should clamp");
    assert_eq!(
        split_ratio_at(&active_tab(&app).root, &[])
            .expect("root should be split")
            .permille(),
        SplitRatio::MAX
    );
}

#[test]
fn pane_headers_consolidate_actions_when_space_is_dense() {
    assert!(pane_header_is_compact(720.0, 1));
    assert!(pane_header_is_compact(1_280.0, 3));
    assert!(!pane_header_is_compact(1_280.0, 2));
}

#[test]
fn the_footer_leaves_its_login_a_usable_lane() {
    // The dot and the collapse control are paid for out of the rail before
    // the login is, so the lane can only shrink when the footer's own
    // anatomy grows. The measured ellipsis will honour whatever is left —
    // including a lane too narrow to say anything — so guard that an
    // ordinary account name still reads in full.
    let settings = AppSettings::default();
    let ordinary =
        "@phoenixmatrix".chars().count() as f32 * settings.ui_pixels(10.0) * UI_TEXT_ADVANCE_RATIO;
    assert!(
        GITHUB_STATUS_LABEL_WIDTH >= ordinary,
        "the footer starved its login: {GITHUB_STATUS_LABEL_WIDTH}px of lane for {ordinary}px of copy"
    );
}

#[test]
fn a_backend_supplied_shell_names_no_program_in_the_header() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    for profile in &mut app.session.profiles {
        profile.name = "WSL shell".into();
        profile.program = String::new();
    }
    assert_eq!(
        app.pane_program(pane_id),
        None,
        "a profile without a program has no command to chip"
    );
    // Copy with room for a fallback still says something truthful.
    assert_eq!(app.pane_command(pane_id), "WSL shell");
}

#[test]
fn agent_events_without_pane_identity_cannot_leak_into_the_focused_session() {
    let mut app = Muxtrix::new();
    app.split_terminal(SplitAxis::Horizontal)
        .expect("second pane should open");
    let focused = active_pane_id(&app);
    app.agent_statuses.insert(
        focused,
        AgentPaneStatus {
            agent: "codex".into(),
            display_name: None,
            state: AgentState::Completed,
            activity: Some("Turn complete".into()),
            session_id: Some("idle-session".into()),
            cwd: None,
            git_branch: None,
        },
    );

    let response = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("UserPromptSubmit".into()),
        title: "Codex · UserPromptSubmit".into(),
        body: "Agent is running".into(),
        pane_id: None,
        session_id: Some("unrelated-session".into()),
        cwd: None,
    });

    assert!(!response.ok);
    assert_eq!(app.agent_statuses[&focused].state, AgentState::Completed);
    assert_eq!(app.pane_state_label(focused), "Idle");
}

#[test]
fn background_agent_notification_marks_and_focus_clears_attention() {
    let mut app = Muxtrix::new();
    let original_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));

    app.record_notification(
        original_pane,
        TerminalNotification {
            title: "Codex".into(),
            body: "Needs approval".into(),
        },
    );

    let pane = app
        .active_workspace()
        .expect("workspace should exist")
        .pane(original_pane)
        .expect("original pane should exist");
    assert_eq!(pane.attention.unread_count, 1);
    assert_eq!(pane.attention.message.as_deref(), Some("Needs approval"));
    assert!(app.notifications[0].unread);

    app.focus_pane(original_pane)
        .expect("notification pane should focus");
    let pane = app
        .active_workspace()
        .expect("workspace should exist")
        .pane(original_pane)
        .expect("original pane should exist");
    assert_eq!(pane.attention.unread_count, 0);
    assert!(!app.notifications[0].unread);
    assert!(app.global_alerts.is_empty());
}

#[test]
fn agent_lifecycle_updates_fleet_without_repainting_workspace_status() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.settings.show_status_bar = true;
    app.status = "Process status".into();

    let running = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("UserPromptSubmit".into()),
        title: "Codex · UserPromptSubmit".into(),
        body: "Agent is running".into(),
        pane_id: Some(pane_id.as_uuid().to_string()),
        session_id: Some("thread-1".into()),
        cwd: Some("/workspace".into()),
    });

    assert!(running.ok);
    assert_eq!(app.status, "Process status");
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);
    assert_eq!(app.pane_activity(pane_id, None), "Agent is running");

    let completed = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Completed,
        event: Some("Stop".into()),
        title: "Codex · Stop".into(),
        body: "Agent completed a turn".into(),
        pane_id: Some(pane_id.as_uuid().to_string()),
        session_id: Some("thread-1".into()),
        cwd: Some("/workspace".into()),
    });

    assert!(completed.ok);
    assert_eq!(app.status, "Process status");
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Completed);
    assert_eq!(app.pane_activity(pane_id, None), "Agent completed a turn");
}

#[test]
fn completed_agent_clears_background_attention_without_creating_more() {
    let mut app = Muxtrix::new();
    let completed_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));

    app.record_notification(
        completed_pane,
        TerminalNotification {
            title: "Codex".into(),
            body: "Needs approval".into(),
        },
    );
    let response = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Completed,
        event: Some("Stop".into()),
        title: "Codex · Stop".into(),
        body: "Turn complete".into(),
        pane_id: Some(completed_pane.as_uuid().to_string()),
        session_id: Some("completed-session".into()),
        cwd: None,
    });

    assert!(response.ok);
    assert_eq!(app.pane_state_label(completed_pane), "Idle");
    let pane = app
        .active_workspace()
        .expect("workspace should exist")
        .pane(completed_pane)
        .expect("completed pane should exist");
    assert_eq!(pane.attention.unread_count, 0);
    assert!(pane.attention.message.is_none());
    assert!(
        app.notifications
            .iter()
            .filter(|notification| notification.pane_id == completed_pane)
            .all(|notification| !notification.unread)
    );
    assert!(
        !app.pane_needs_attention(completed_pane, 1),
        "a completed turn must remain neutral even if stale unread data is restored"
    );
}

#[test]
fn a_completed_turn_reads_as_idle_without_losing_its_internal_state() {
    assert_eq!(
        agent_state_label(AgentState::Completed),
        agent_state_label(AgentState::Idle),
        "a finished turn and an untouched composer both read as Idle"
    );
    assert_eq!(agent_state_label(AgentState::Completed), "Idle");

    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "codex".into(),
            display_name: None,
            state: AgentState::Completed,
            activity: None,
            session_id: None,
            cwd: None,
            git_branch: None,
        },
    );
    assert_eq!(app.pane_state_label(pane_id), "Idle");
    // Only the word is shared. The state itself still decides attention and
    // the signal a finished turn wears, so it must survive the relabel.
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Completed);
    assert!(!app.pane_needs_attention(pane_id, 1));
    assert_eq!(app.pane_signal_kind(pane_id, true), PaneSignalKind::Neutral);
}

#[test]
fn completed_agent_stays_done_until_a_working_screen_starts_the_next_turn() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "codex".into(),
            display_name: None,
            state: AgentState::Completed,
            activity: Some("Turn complete".into()),
            session_id: Some("thread-1".into()),
            cwd: None,
            git_branch: None,
        },
    );

    app.apply_agent_screen_classification(
        pane_id,
        "codex",
        1,
        agent_screen::Classification {
            state: agent_screen::ScreenState::Idle,
            rule: "codex.live_prompt",
        },
    );
    assert_eq!(
        app.agent_statuses[&pane_id].state,
        AgentState::Completed,
        "an idle composer should preserve the completed-turn signal"
    );

    app.apply_agent_screen_classification(
        pane_id,
        "codex",
        2,
        agent_screen::Classification {
            state: agent_screen::ScreenState::Running,
            rule: "codex.osc_title_running",
        },
    );
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);
    assert_eq!(
        app.agent_statuses[&pane_id].activity.as_deref(),
        Some("Agent is working")
    );
    assert_eq!(app.agent_running_frame_revisions.get(&pane_id), Some(&2));
}

#[test]
fn pi_idle_title_does_not_override_an_active_lifecycle() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let pane = Some(pane_id.as_uuid().to_string());
    let running_revision = app.terminals[&pane_id].snapshot_revision;

    let started = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "pi".into(),
        state: AgentState::Running,
        event: Some("agent_start".into()),
        title: "Oh My Pi".into(),
        body: "Agent is running".into(),
        pane_id: pane.clone(),
        session_id: Some("session-1".into()),
        cwd: None,
    });
    assert!(started.ok);

    app.apply_agent_screen_classification(
        pane_id,
        "pi",
        running_revision.wrapping_add(1),
        agent_screen::Classification {
            state: agent_screen::ScreenState::Idle,
            rule: "pi.osc_title_idle",
        },
    );
    assert_eq!(
        app.agent_statuses[&pane_id].state,
        AgentState::Running,
        "a stale Pi idle title must not override an active agent lifecycle"
    );

    let compacted = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "pi".into(),
        state: AgentState::Completed,
        event: Some("session_compact".into()),
        title: "Oh My Pi".into(),
        body: "Context compacted".into(),
        pane_id: pane.clone(),
        session_id: Some("session-1".into()),
        cwd: None,
    });
    assert!(compacted.ok);
    assert_eq!(
        app.agent_statuses[&pane_id].state,
        AgentState::Running,
        "an older managed extension's maintenance completion must preserve the active run"
    );

    let ended = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "pi".into(),
        state: AgentState::Completed,
        event: Some("agent_end".into()),
        title: "Oh My Pi".into(),
        body: "Agent completed a turn".into(),
        pane_id: pane,
        session_id: Some("session-1".into()),
        cwd: None,
    });
    assert!(ended.ok);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Completed);
}

#[test]
fn newer_pi_idle_title_clears_a_stale_running_lifecycle() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "pi".into(),
            display_name: None,
            state: AgentState::Running,
            activity: Some("Agent is running".into()),
            session_id: Some("session-1".into()),
            cwd: None,
            git_branch: None,
        },
    );
    app.agent_running_frame_revisions.insert(pane_id, 10);

    let idle = agent_screen::Classification {
        state: agent_screen::ScreenState::Idle,
        rule: "pi.osc_title_idle",
    };
    app.apply_agent_screen_classification(pane_id, "pi", 10, idle);
    assert_eq!(
        app.agent_statuses[&pane_id].state,
        AgentState::Running,
        "the frame painted before the running event remains race-guarded"
    );

    app.apply_agent_screen_classification(pane_id, "pi", 11, idle);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Idle);
    assert_eq!(
        app.agent_statuses[&pane_id].activity.as_deref(),
        Some("Ready for input")
    );
    assert!(!app.agent_running_frame_revisions.contains_key(&pane_id));
}

#[test]
fn isolated_hook_discovery_is_read_only() {
    assert!(hook_discovery_may_migrate_paths(false, false, false));
    assert!(!hook_discovery_may_migrate_paths(true, false, false));
    assert!(!hook_discovery_may_migrate_paths(false, true, false));
    assert!(!hook_discovery_may_migrate_paths(false, false, true));
    assert!(!hook_discovery_may_migrate_paths(true, true, true));
}

#[test]
fn codex_auto_approval_hooks_never_create_attention() {
    let mut app = Muxtrix::new();
    let original_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let pane = Some(original_pane.as_uuid().to_string());

    for request_number in 1..=3 {
        let waiting = app.handle_control_request(ControlRequest::AgentEvent {
            agent: "codex".into(),
            state: AgentState::Waiting,
            event: Some("PermissionRequest".into()),
            title: "Codex · PermissionRequest".into(),
            body: format!("Tool {request_number} needs approval"),
            pane_id: pane.clone(),
            session_id: Some("thread-1".into()),
            cwd: None,
        });
        assert!(waiting.ok);
        assert_eq!(
            app.active_workspace()
                .expect("workspace should exist")
                .pane(original_pane)
                .expect("original pane should exist")
                .attention
                .unread_count,
            0,
            "a request sent to the automatic reviewer is not user attention"
        );
        assert_eq!(
            app.agent_statuses[&original_pane].state,
            AgentState::Running
        );

        let resumed = app.handle_control_request(ControlRequest::AgentEvent {
            agent: "codex".into(),
            state: AgentState::Running,
            event: Some("PostToolUse".into()),
            title: "Codex · PostToolUse".into(),
            body: format!("Tool {request_number} finished"),
            pane_id: pane.clone(),
            session_id: Some("thread-1".into()),
            cwd: None,
        });
        assert!(resumed.ok);
        assert_eq!(
            app.agent_statuses[&original_pane].state,
            AgentState::Running
        );
        assert_eq!(
            app.active_workspace()
                .expect("workspace should exist")
                .pane(original_pane)
                .expect("original pane should exist")
                .attention
                .unread_count,
            0,
            "automatic approval cycles must not accumulate attention"
        );
        assert!(
            app.notifications
                .iter()
                .filter(|notification| notification.pane_id == original_pane)
                .all(|notification| !notification.unread)
        );
    }
}

#[test]
fn visible_prompt_owns_attention_until_the_screen_resolves_it() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let pane = Some(pane_id.as_uuid().to_string());
    let start = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("UserPromptSubmit".into()),
        title: "Codex · UserPromptSubmit".into(),
        body: "Working".into(),
        pane_id: pane.clone(),
        session_id: Some("thread-1".into()),
        cwd: None,
    });
    assert!(start.ok);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));

    app.apply_agent_screen_classification(
        pane_id,
        "codex",
        1,
        agent_screen::Classification {
            state: agent_screen::ScreenState::Waiting,
            rule: "codex.live_strong_blocker",
        },
    );
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Waiting);
    assert_eq!(
        app.active_workspace()
            .expect("workspace should exist")
            .pane(pane_id)
            .expect("pane should exist")
            .attention
            .unread_count,
        1,
        "a visible prompt in a background pane should create attention"
    );

    let post_tool = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("PostToolUse".into()),
        title: "Codex · PostToolUse".into(),
        body: "A parallel tool finished".into(),
        pane_id: pane,
        session_id: Some("thread-1".into()),
        cwd: None,
    });
    assert!(post_tool.ok);
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Waiting);
    assert_eq!(
        app.active_workspace()
            .expect("workspace should exist")
            .pane(pane_id)
            .expect("pane should exist")
            .attention
            .unread_count,
        1,
        "late tool output must not clear the visible prompt"
    );

    app.apply_agent_screen_classification(
        pane_id,
        "codex",
        2,
        agent_screen::Classification {
            state: agent_screen::ScreenState::Running,
            rule: "codex.osc_title_spinner",
        },
    );
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);
    assert_eq!(
        app.active_workspace()
            .expect("workspace should exist")
            .pane(pane_id)
            .expect("pane should exist")
            .attention
            .unread_count,
        0,
        "working screen evidence should clear the prompt attention"
    );
}

#[test]
fn retained_idle_screen_only_yields_to_a_newer_frame() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let pane = Some(pane_id.as_uuid().to_string());
    let running_revision = app.terminals[&pane_id].snapshot_revision;
    let start = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Running,
        event: Some("UserPromptSubmit".into()),
        title: "Codex · UserPromptSubmit".into(),
        body: "Working".into(),
        pane_id: pane,
        session_id: Some("thread-1".into()),
        cwd: None,
    });
    assert!(start.ok);

    app.apply_agent_screen_classification(
        pane_id,
        "codex",
        running_revision,
        agent_screen::Classification {
            state: agent_screen::ScreenState::Idle,
            rule: "codex.osc_title_idle",
        },
    );

    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Running);
    assert_eq!(
        app.agent_statuses[&pane_id].activity.as_deref(),
        Some("Working")
    );

    app.apply_agent_screen_classification(
        pane_id,
        "codex",
        running_revision.wrapping_add(1),
        agent_screen::Classification {
            state: agent_screen::ScreenState::Idle,
            rule: "codex.osc_title_idle",
        },
    );
    assert_eq!(app.agent_statuses[&pane_id].state, AgentState::Idle);
    assert_eq!(
        app.agent_statuses[&pane_id].activity.as_deref(),
        Some("Ready for input")
    );
}

#[test]
fn redesigned_workspace_chrome_is_stateful_without_affecting_terminals() {
    let mut app = Muxtrix::new();
    assert!(!app.sidebar_collapsed);
    assert!(app.maximized_pane.is_none());
    assert!(!app.settings.show_status_bar);

    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let focused = active_pane_id(&app);
    let workspace = app.active_workspace().expect("workspace should exist");
    let tab = workspace.active_tab().expect("active tab should exist");
    let titles: Vec<_> = pane_ids_in_layout(&tab.root)
        .into_iter()
        .filter_map(|pane_id| {
            tab.panes[&pane_id]
                .active_surface()
                .map(|surface| surface.title.as_str())
        })
        .collect();
    assert_eq!(titles, vec!["shell 1", "shell 2"]);

    let _ = app.update(Message::ToggleSidebar);
    let _ = app.update(Message::ToggleMaximize(focused));
    let _ = app.update(Message::TogglePaneMenu(focused));
    assert!(app.sidebar_collapsed);
    assert_eq!(app.maximized_pane, Some(focused));
    assert_eq!(app.pane_menu, Some(focused));
    assert_eq!(app.terminals.len(), 2);

    let _ = app.update(Message::ToggleMaximize(focused));
    let _ = app.update(Message::TogglePaneMenu(focused));
    assert!(app.maximized_pane.is_none());
    assert!(app.pane_menu.is_none());
}

#[test]
fn pane_menu_dismisses_without_reaching_the_terminal() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);

    let _ = app.update(Message::TogglePaneMenu(pane_id));
    let _ = app.update(Message::DismissPaneMenu);
    assert!(app.pane_menu.is_none());

    let _ = app.update(Message::TogglePaneMenu(pane_id));
    let _ = app.handle_keyboard(key_press(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(app.pane_menu.is_none());
    assert!(app.pending_terminal_input.is_empty());

    let _ = app.update(Message::TogglePaneMenu(pane_id));
    let _ = app.update(Message::ToggleMaximizeFromPaneMenu(pane_id));
    assert!(app.pane_menu.is_none());
    assert_eq!(app.maximized_pane, Some(pane_id));
}

#[test]
fn maximized_pane_blocks_hidden_layout_mutations() {
    let mut app = Muxtrix::new();
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let pane_id = active_pane_id(&app);
    let pane_count = app
        .active_workspace()
        .expect("workspace should exist")
        .active_tab()
        .expect("active tab should exist")
        .panes
        .len();

    let _ = app.update(Message::ToggleMaximize(pane_id));
    assert_eq!(app.maximized_pane, Some(pane_id));

    let _ = app.update(Message::Split(SplitAxis::Vertical));
    assert_eq!(
        app.active_workspace()
            .expect("workspace should exist")
            .active_tab()
            .expect("active tab should exist")
            .panes
            .len(),
        pane_count
    );
    assert_eq!(
        app.status,
        "Restore panes before splitting the focused pane"
    );

    app.palette.visible = true;
    app.palette.query = "split".into();
    app.palette.selected = 0;
    let _ = app.update(Message::CommandSelected(0));
    assert!(
        app.palette.visible,
        "a disabled palette row must not execute"
    );
    assert_eq!(
        app.active_workspace()
            .expect("workspace should exist")
            .active_tab()
            .expect("active tab should exist")
            .panes
            .len(),
        pane_count
    );

    let _ = app.update(Message::CommandQueryChanged(String::new()));
    let commands = commands::filtered("");
    assert!(app.command_enabled(commands[app.palette.selected].action));
    assert!(
        app.palette.selected > 0,
        "disabled leading rows are skipped"
    );
}

#[test]
fn app_workspaces_keep_independent_terminal_fleets() {
    let mut app = Muxtrix::new();
    let first_workspace = app.session.active_workspace_id;
    let first_pane = active_pane_id(&app);

    create_test_workspace(&mut app);
    let second_workspace = app.session.active_workspace_id;
    let second_pane = active_pane_id(&app);
    assert_ne!(first_workspace, second_workspace);
    assert_ne!(first_pane, second_pane);
    assert_eq!(app.terminals.len(), 2);

    app.switch_workspace(first_workspace)
        .expect("first workspace should switch");
    assert_eq!(active_pane_id(&app), first_pane);
    app.close_workspace().expect("first workspace should close");
    assert_eq!(app.session.active_workspace_id, second_workspace);
    assert!(!app.terminals.contains_key(&first_pane));
    assert!(app.terminals.contains_key(&second_pane));
}

#[test]
fn tabs_start_with_one_pane_and_last_pane_closes_the_tab() {
    let mut app = Muxtrix::new();
    let original_tab = active_tab(&app).id;
    let original_pane = active_pane_id(&app);

    app.new_tab().expect("tab should be created");
    let new_tab = active_tab(&app).id;
    let new_pane = active_pane_id(&app);
    assert_ne!(original_tab, new_tab);
    assert_eq!(active_tab(&app).panes.len(), 1);
    assert!(app.terminals.contains_key(&new_pane));

    app.close_pane(new_pane)
        .expect("closing the tab root pane should close its tab");
    let workspace = app.active_workspace().expect("workspace should exist");
    assert_eq!(workspace.tabs.len(), 1);
    assert_eq!(workspace.active_tab_id, original_tab);
    assert!(app.terminals.contains_key(&original_pane));
    assert!(!app.terminals.contains_key(&new_pane));
}

#[test]
fn closing_the_final_tab_requests_workspace_confirmation() {
    let mut app = Muxtrix::new();
    let workspace_id = app.session.active_workspace_id;
    let pane_id = active_pane_id(&app);

    app.close_pane(pane_id)
        .expect("last pane should request confirmation");

    assert_eq!(app.close_workspace_prompt, Some(workspace_id));
    assert_eq!(app.session.workspaces.len(), 1);
    assert_eq!(app.session.workspaces[0].tabs.len(), 1);
    assert!(app.terminals.contains_key(&pane_id));
}

#[test]
fn tab_drag_reorders_and_moves_tabs_between_workspaces() {
    let mut app = Muxtrix::new();
    let first_workspace = app.session.active_workspace_id;
    let first_tab = active_tab(&app).id;
    app.new_tab().expect("second tab should be created");
    let moved_tab = active_tab(&app).id;
    app.tab_drag = Some(TabDrag {
        tab_id: moved_tab,
        target_workspace_id: first_workspace,
        target_index: 0,
    });
    app.finish_tab_drag().expect("tab should reorder");
    assert_eq!(app.session.workspaces[0].tabs[0].id, moved_tab);
    assert_eq!(app.session.workspaces[0].tabs[1].id, first_tab);

    create_test_workspace(&mut app);
    let second_workspace = app.session.active_workspace_id;
    app.tab_drag = Some(TabDrag {
        tab_id: moved_tab,
        target_workspace_id: second_workspace,
        target_index: 0,
    });
    app.finish_tab_drag()
        .expect("tab should move between workspaces");
    assert_eq!(app.session.active_workspace_id, second_workspace);
    assert_eq!(app.session.workspaces[0].tabs.len(), 1);
    assert_eq!(app.session.workspaces[0].tabs[0].id, first_tab);
    assert_eq!(app.session.workspaces[1].tabs[0].id, moved_tab);
}

#[test]
fn windows_shell_setting_builds_native_and_wsl_profiles() {
    let mut settings = AppSettings::default();
    let native = windows_profile(&settings, ProfileId::new());
    assert_eq!(native.backend, ProcessBackend::Local);
    assert_eq!(native.program, "powershell.exe");

    settings.windows_shell_backend = WindowsShellBackend::Wsl;
    settings.wsl_distribution = " Ubuntu-24.04 ".into();
    let wsl = windows_profile(&settings, ProfileId::new());
    assert_eq!(
        wsl.backend,
        ProcessBackend::Wsl {
            distribution: Some("Ubuntu-24.04".into())
        }
    );
    assert!(wsl.program.is_empty());
    assert!(wsl.arguments.is_empty());
    assert_eq!(wsl.working_directory, Some("~".into()));
}

#[test]
fn wsl_registry_discovery_filters_utility_distros_and_duplicates() {
    assert_eq!(
        visible_wsl_distribution_names([
            "Ubuntu-24.04".into(),
            "docker-desktop".into(),
            "Debian".into(),
            "ubuntu-24.04".into(),
            "rancher-desktop-data".into(),
        ]),
        ["Debian", "Ubuntu-24.04"]
    );
}

#[test]
fn fleet_scope_switches_between_current_and_grouped_workspaces() {
    let mut app = Muxtrix::new();
    let first = active_pane_id(&app);
    let first_workspace = app.session.active_workspace_id;
    app.agent_statuses
        .insert(first, agent_status("claude-code"));
    create_test_workspace(&mut app);
    let second = active_pane_id(&app);
    let second_workspace = app.session.active_workspace_id;
    app.agent_statuses.insert(second, agent_status("codex"));

    assert_eq!(app.settings.fleet_scope, FleetScope::CurrentWorkspace);
    let _ = app.update(Message::SetFleetView(FleetView::Agents));
    assert_eq!(app.fleet_entries(), vec![(second_workspace, second)]);

    app.set_fleet_scope(FleetScope::AllWorkspaces);
    assert_eq!(app.settings_draft.fleet_scope, FleetScope::AllWorkspaces);
    assert_eq!(
        app.fleet_entries(),
        vec![(first_workspace, first), (second_workspace, second)]
    );
    assert_eq!(
        app.rail_targets()
            .into_iter()
            .filter(|target| !matches!(target, RailTarget::Workspace(_)))
            .collect::<Vec<_>>(),
        vec![
            RailTarget::FleetWorkspace(first_workspace),
            RailTarget::FleetPane(first_workspace, first),
            RailTarget::FleetWorkspace(second_workspace),
            RailTarget::FleetPane(second_workspace, second),
        ],
        "all-workspaces mode groups the visible agent panes by workspace"
    );

    let _ = app.update(Message::SetFleetView(FleetView::Tabs));
    let _ = app.update(Message::SwitchWorkspace(first_workspace));
    assert_eq!(
        app.fleet_entries(),
        vec![(first_workspace, first), (second_workspace, second)],
        "switching the active workspace must not narrow all-workspaces mode"
    );

    app.set_fleet_scope(FleetScope::CurrentWorkspace);
    assert_eq!(app.fleet_entries(), vec![(first_workspace, first)]);
}

#[test]
fn fleet_palette_command_toggles_workspace_visibility() {
    let mut app = Muxtrix::new();

    drop(app.run_command(CommandAction::FleetToggleAllWorkspaces));
    assert_eq!(app.settings.fleet_scope, FleetScope::AllWorkspaces);
    assert_eq!(app.settings_draft.fleet_scope, FleetScope::AllWorkspaces);
    assert_eq!(app.status, "Fleet shows all workspaces");

    drop(app.run_command(CommandAction::FleetToggleAllWorkspaces));
    assert_eq!(app.settings.fleet_scope, FleetScope::CurrentWorkspace);
    assert_eq!(app.settings_draft.fleet_scope, FleetScope::CurrentWorkspace);
    assert_eq!(app.status, "Fleet shows only the current workspace");
}

fn agent_status(agent: &str) -> AgentPaneStatus {
    AgentPaneStatus {
        agent: agent.into(),
        display_name: None,
        state: AgentState::Running,
        activity: None,
        session_id: None,
        cwd: None,
        git_branch: None,
    }
}

#[test]
fn a_pane_projecting_the_roster_reports_the_roster_not_its_own_state() {
    let mut app = Muxtrix::new();
    let pane = active_pane_id(&app);
    // The conversation behind the roster is backgrounded; whatever state it
    // last held must not be what the row shows.
    app.agent_statuses.insert(pane, agent_status("claude-code"));
    assert_eq!(app.pane_state_label(pane), "Running");

    app.agents_view_panes.insert(pane);
    // Before the first read the count is genuinely unknown.
    assert_eq!(app.pane_state_label(pane), "Agents");

    app.agents_roster = Some(agents_roster::AgentsRoster {
        working: 3,
        blocked: 0,
        failed: 0,
        completed: 1,
        idle: 0,
    });
    assert_eq!(app.pane_state_label(pane), "3 working");
    assert_eq!(app.pane_signal_kind(pane, false), PaneSignalKind::Active);
    assert_eq!(app.pane_activity(pane, None), "3 working · 1 idle");

    // Leaving the view hands the row straight back to the pane's own state.
    app.agents_view_panes.remove(&pane);
    assert_eq!(app.pane_state_label(pane), "Running");
}

/// The state a healthy fleet spends most of its time in still has to reach
/// the row: a finished roster reads like a finished agent, never like a
/// pane with nothing to say.
#[test]
fn a_finished_roster_still_reports_a_state_and_a_signal() {
    let mut app = Muxtrix::new();
    let pane = active_pane_id(&app);
    app.agent_statuses.insert(pane, agent_status("claude-code"));
    app.agents_view_panes.insert(pane);
    app.agents_roster = Some(agents_roster::AgentsRoster {
        working: 0,
        blocked: 0,
        failed: 0,
        completed: 4,
        idle: 2,
    });
    // Six sessions are resting; naming only the four that finished would be
    // untrue now that both halves wear the same word.
    assert_eq!(app.pane_state_label(pane), "6 idle");
    assert_eq!(app.pane_signal_kind(pane, false), PaneSignalKind::Neutral);
    assert_eq!(app.pane_activity(pane, None), "6 idle");
}

#[test]
fn a_blocked_member_raises_the_rosters_row_to_human_attention() {
    let mut app = Muxtrix::new();
    let pane = active_pane_id(&app);
    app.agent_statuses.insert(pane, agent_status("claude-code"));
    app.agents_view_panes.insert(pane);
    app.agents_roster = Some(agents_roster::AgentsRoster {
        working: 3,
        blocked: 1,
        failed: 0,
        completed: 0,
        idle: 0,
    });
    assert_eq!(app.pane_state_label(pane), "1 needs input");
    assert_eq!(app.pane_signal_kind(pane, false), PaneSignalKind::Warning);
}

/// A roll-up that cannot run is a fact about Muxtrix, not about the fleet.
/// The row says so instead of waiting on a read that will never land.
#[test]
fn a_roster_that_cannot_be_read_says_so_instead_of_waiting_forever() {
    let mut app = Muxtrix::new();
    let pane = active_pane_id(&app);
    app.agent_statuses.insert(pane, agent_status("claude-code"));
    app.agents_view_panes.insert(pane);
    assert_eq!(app.pane_state_label(pane), "Agents");

    let _ = app.update(Message::AgentsRosterLoaded(Err(
        "could not run `claude agents --json`: not found".into(),
    )));
    assert_eq!(app.pane_state_label(pane), "Unavailable");
    assert!(app.pane_activity(pane, None).contains("not found"));

    // A later read that lands replaces the reason with the counts.
    let _ = app.update(Message::AgentsRosterLoaded(Ok(
        agents_roster::AgentsRoster {
            working: 1,
            blocked: 0,
            failed: 0,
            completed: 0,
            idle: 0,
        },
    )));
    assert_eq!(app.pane_state_label(pane), "1 working");
    assert_eq!(app.pane_activity(pane, None), "1 working");
}

#[test]
fn an_empty_roster_is_reported_rather_than_left_blank() {
    let mut app = Muxtrix::new();
    let pane = active_pane_id(&app);
    app.agent_statuses.insert(pane, agent_status("claude-code"));
    app.agents_view_panes.insert(pane);
    app.agents_roster = Some(agents_roster::AgentsRoster::default());
    assert_eq!(app.pane_state_label(pane), "No agents");
    assert_eq!(app.pane_signal_kind(pane, false), PaneSignalKind::Neutral);
}

#[test]
fn toggling_into_the_roster_and_back_never_renames_a_fleet_row() {
    // Claude Code retitles the pane on the way in and on the way out. The
    // row must keep the identity its work earned.
    assert_eq!(
        harness_terminal_title("◐ Port the idle rule", "claude-code"),
        Some("Port the idle rule".into())
    );
    for chrome in [
        "claude agents",
        "2 awaiting input · claude agents",
        "current session",
    ] {
        assert_eq!(
            harness_terminal_title(chrome, "claude-code"),
            None,
            "{chrome}"
        );
    }
}

#[test]
fn agents_view_keeps_tab_order_and_never_drops_unrecognized_agents() {
    let mut app = Muxtrix::new();
    let claude_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let unknown_pane = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    let codex_pane = active_pane_id(&app);
    app.agent_statuses
        .insert(claude_pane, agent_status("claude-code"));
    app.agent_statuses
        .insert(unknown_pane, agent_status("gemini"));
    app.agent_statuses.insert(codex_pane, agent_status("codex"));

    assert_eq!(agent_display_name("claude-code"), "Claude Code");

    app.set_fleet_view(FleetView::Agents);
    assert_eq!(app.settings_draft.fleet_view, FleetView::Agents);
    let workspace_id = app.session.active_workspace_id;
    let pane_order = pane_ids_in_layout(&active_tab(&app).root);
    assert!(pane_order.contains(&unknown_pane));
    let expected_entries = pane_order
        .iter()
        .map(|pane_id| (workspace_id, *pane_id))
        .collect::<Vec<_>>();
    assert_eq!(
        app.fleet_entries(),
        expected_entries,
        "agent type must not regroup panes away from their tab order"
    );
    assert_eq!(
        app.rail_targets()
            .into_iter()
            .filter(|target| !matches!(target, RailTarget::Workspace(_)))
            .collect::<Vec<_>>(),
        pane_order
            .into_iter()
            .map(|pane_id| RailTarget::FleetPane(workspace_id, pane_id))
            .collect::<Vec<_>>(),
        "rail navigation follows the visible flat agent order without hidden bands"
    );
}

#[test]
fn collapsed_rail_ignores_expanded_fleet_projections() {
    let mut app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let _ = app.update(Message::Split(SplitAxis::Horizontal));
    app.agent_statuses.insert(
        pane_id,
        AgentPaneStatus {
            agent: "claude".into(),
            display_name: None,
            state: AgentState::Running,
            activity: None,
            session_id: None,
            cwd: None,
            git_branch: None,
        },
    );
    let _ = app.update(Message::SetFleetView(FleetView::Agents));
    assert_eq!(app.fleet_entries().len(), 1);

    create_test_workspace(&mut app);
    app.set_fleet_scope(FleetScope::AllWorkspaces);

    // The collapsed rail has no reachable projection toggle, so it lists
    // every pane in the selected scope without hidden group targets.
    app.sidebar_collapsed = true;
    assert_eq!(app.fleet_entries().len(), 3);
    let fleet_targets = app
        .rail_targets()
        .into_iter()
        .filter(|target| !matches!(target, RailTarget::Workspace(_)))
        .collect::<Vec<_>>();
    assert_eq!(fleet_targets.len(), 3);
    assert!(
        fleet_targets
            .iter()
            .all(|target| matches!(target, RailTarget::FleetPane(_, _)))
    );

    app.set_fleet_view(FleetView::Repos);
    assert_eq!(app.fleet_entries().len(), 3);
}

#[test]
fn shell_rows_report_their_launch_command() {
    let app = Muxtrix::new();
    let pane_id = active_pane_id(&app);
    let command = app.pane_command(pane_id);
    assert!(!command.is_empty());
    assert!(
        !command.contains('/'),
        "the command should be a basename or profile name, not a path: {command}"
    );
}

#[test]
fn typed_control_requests_manage_panes_and_agent_attention() {
    let mut app = Muxtrix::new();
    let original = active_pane_id(&app);
    let response = app.handle_control_request(ControlRequest::Split {
        direction: SplitDirection::Right,
    });
    assert!(response.ok);
    let second = active_pane_id(&app);
    assert_ne!(original, second);

    let response = app.handle_control_request(ControlRequest::AgentEvent {
        agent: "codex".into(),
        state: AgentState::Waiting,
        event: Some("PermissionRequest".into()),
        title: "Codex · PermissionRequest".into(),
        body: "Codex needs approval".into(),
        pane_id: Some(original.as_uuid().to_string()),
        session_id: Some("thread-1".into()),
        cwd: Some("/workspace".into()),
    });
    assert!(response.ok);
    assert_eq!(
        app.active_workspace()
            .expect("workspace should exist")
            .pane(original)
            .expect("original pane should exist")
            .attention
            .unread_count,
        0
    );
    assert_eq!(app.agent_statuses[&original].state, AgentState::Running);
    assert_eq!(app.pane_activity(original, None), "Codex needs approval");
    assert_ne!(app.pane_activity(original, None), "Ready for input");

    let response = app.handle_control_request(ControlRequest::ListPanes);
    assert!(response.ok);
    assert_eq!(response.panes.len(), 2);
    assert!(response.panes.iter().any(|pane| pane.focused));

    let response = app.handle_control_request(ControlRequest::Close {
        pane_id: Some(original.as_uuid().to_string()),
    });
    assert!(response.ok);
    assert_eq!(
        app.active_workspace()
            .expect("workspace should exist")
            .active_tab()
            .expect("active tab should exist")
            .panes
            .len(),
        1
    );
    assert!(!app.terminals.contains_key(&original));
}

#[test]
fn wsl_sessions_share_pane_identity_with_windows_hook_processes() {
    let pane_id = PaneId::new();
    let mut plan = LaunchPlan {
        executable: "wsl.exe".into(),
        arguments: Vec::new(),
        working_directory: None,
        environment: Vec::new(),
    };
    add_muxtrix_environment(
        &mut plan,
        &ProcessBackend::Wsl { distribution: None },
        pane_id,
        Some("EXISTING/p:MUXTRIX_PANE_ID/u"),
        Some("muxtrix-test-endpoint"),
        Some("/home/user/.local/share/muxtrix/shell-integration/zsh"),
    );

    assert!(
        plan.environment
            .contains(&("MUXTRIX_PANE_ID".into(), pane_id.as_uuid().to_string()))
    );
    assert!(plan.environment.contains(&(
        "WSLENV".into(),
        "EXISTING/p:MUXTRIX_PANE_ID/u:MUXTRIX_CONTROL_ENDPOINT:PROMPT_COMMAND:ZDOTDIR".into()
    )));
    assert!(plan.environment.contains(&(
        "ZDOTDIR".into(),
        "/home/user/.local/share/muxtrix/shell-integration/zsh".into()
    )));
    assert!(
        plan.environment
            .iter()
            .any(|(name, value)| name == "PROMPT_COMMAND" && value.contains("]7;file://"))
    );
    assert!(plan.environment.contains(&(
        "MUXTRIX_CONTROL_ENDPOINT".into(),
        "muxtrix-test-endpoint".into()
    )));
}

#[test]
fn native_sessions_receive_the_exact_control_endpoint_too() {
    let pane_id = PaneId::new();
    let mut plan = LaunchPlan {
        executable: "powershell.exe".into(),
        arguments: Vec::new(),
        working_directory: None,
        environment: Vec::new(),
    };
    add_muxtrix_environment(
        &mut plan,
        &ProcessBackend::Local,
        pane_id,
        None,
        Some("muxtrix-native-endpoint"),
        None,
    );

    assert!(plan.environment.contains(&(
        "MUXTRIX_CONTROL_ENDPOINT".into(),
        "muxtrix-native-endpoint".into()
    )));
    assert!(!plan.environment.iter().any(|(name, _)| name == "WSLENV"));
}
