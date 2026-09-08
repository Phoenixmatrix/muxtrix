//! Asynchronous task checkout creation and explicitly confirmed completion.
use super::*;
use muxtrix_domain::TaskWorktree;

#[derive(Debug, Clone)]
pub(crate) struct CompleteTaskPrompt {
    pub(crate) pane_id: PaneId,
    pub(crate) warning: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskPaneCreation {
    pub(crate) id: uuid::Uuid,
    epoch: u64,
    workspace_id: WorkspaceId,
    tab_id: TabId,
    source_id: PaneId,
    profile: LaunchProfile,
    agent: Agent,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskCompletion {
    pub(crate) id: uuid::Uuid,
    epoch: u64,
    pane_id: PaneId,
    task: TaskWorktree,
    pub(crate) removing: bool,
}

/// Retains the stop obligation after a timeout or scheduling failure. Dropping
/// UI state must never join an actor which may still be processing that stop.
pub(crate) struct TaskTerminalStop {
    pub(crate) session: Mutex<Option<LiveSession>>,
    pub(crate) pending: AtomicBool,
}

impl Drop for TaskTerminalStop {
    fn drop(&mut self) {
        if let Ok(session) = self.session.get_mut()
            && let Some(session) = session.take()
        {
            dispose_live_session(session);
        }
    }
}

impl Muxtrix {
    pub(crate) fn task_worktree(&self, pane_id: PaneId) -> Option<&TaskWorktree> {
        self.session
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane(pane_id))
            .and_then(|pane| pane.task_worktree.as_ref())
    }

    pub(crate) fn create_task_pane(&mut self) -> Vec<Effect> {
        if self.pending_task_creation.is_some() || self.task_removal_busy() {
            return Vec::new();
        }
        let Some(agent) = self.default_agent_for_worktree_command(CommandAction::CreateTaskPane)
        else {
            return Vec::new();
        };
        let result = (|| {
            let workspace = self.active_workspace()?;
            let tab = workspace.active_tab().ok_or("Active tab is missing")?;
            let source_id = tab.focused_pane_id;
            let directory = self
                .pane_terminal_directory(source_id)
                .filter(|path| reported_path_is_concrete(path))
                .ok_or("The source pane has no local working directory")?;
            let profile = self
                .pane_profile(source_id)
                .cloned()
                .ok_or("Terminal profile is missing")?;
            let distribution = match &profile.backend {
                ProcessBackend::Wsl { distribution } => distribution.clone().unwrap_or_default(),
                ProcessBackend::Local => String::new(),
            };
            Ok::<_, String>((
                TaskPaneCreation {
                    id: uuid::Uuid::new_v4(),
                    epoch: self.task_session_epoch,
                    workspace_id: workspace.id,
                    tab_id: tab.id,
                    source_id,
                    profile,
                    agent,
                },
                directory,
                distribution,
            ))
        })();
        let (request, directory, distribution) = match result {
            Ok(request) => request,
            Err(error) => {
                self.global_alerts.push(GlobalAlert {
                    title: "Task creation failed".into(),
                    body: error.clone(),
                });
                self.status = error;
                return Vec::new();
            }
        };
        self.pending_task_creation = Some(request.clone());
        self.active_view = ActiveView::Workspace;
        self.status = "Creating task worktree…".into();
        perform_blocking(
            move || crate::task_worktree::create(&directory, &distribution),
            move |result| {
                Message::TaskPaneCreated(Box::new((
                    request,
                    result.and_then(std::convert::identity),
                )))
            },
        )
    }

    pub(crate) fn finish_task_creation(
        &mut self,
        request: TaskPaneCreation,
        result: Result<TaskWorktree, String>,
    ) {
        let current = self
            .pending_task_creation
            .as_ref()
            .is_some_and(|pending| pending.id == request.id);
        if current {
            self.pending_task_creation = None;
        }
        let task = match result {
            Ok(task) => task,
            Err(error) => {
                self.global_alerts.push(GlobalAlert {
                    title: "Task creation failed".into(),
                    body: error.clone(),
                });
                self.status = error;
                return;
            }
        };
        let target_exists = current
            && request.epoch == self.task_session_epoch
            && self
                .session
                .workspaces
                .iter()
                .find(|workspace| workspace.id == request.workspace_id)
                .and_then(|workspace| workspace.tabs.iter().find(|tab| tab.id == request.tab_id))
                .is_some_and(|tab| tab.panes.contains_key(&request.source_id));
        if !target_exists {
            self.report_retained_task(&task, "The original pane or session is no longer available");
            return;
        }
        let mut profile = request.profile;
        profile.working_directory = Some(task.path.clone());
        let title = task.branch.clone();
        let surface = Surface::terminal(
            &title,
            TerminalSurface {
                profile_id: profile.id,
                working_directory: Some(task.path.clone()),
            },
        );
        let workspace = self
            .session
            .workspaces
            .iter()
            .find(|workspace| workspace.id == request.workspace_id)
            .expect("task workspace checked above");
        let tab = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == request.tab_id)
            .expect("task tab checked above");
        let focus_task = self.session.active_workspace_id == request.workspace_id
            && workspace.active_tab_id == request.tab_id
            && tab.focused_pane_id == request.source_id;
        let placement = if self.maximized_pane.is_some() {
            None
        } else {
            crate::layout::task_pane_placement(
                tab,
                request.source_id,
                |pane_id| {
                    self.terminals
                        .get(&pane_id)
                        .and_then(|runtime| runtime.viewport)
                },
                &self.settings,
            )
        };
        if placement.is_some() {
            self.clear_manual_layout_history(request.tab_id);
        }
        let workspace = self
            .session
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == request.workspace_id)
            .expect("task workspace checked above");
        let inserted = if let Some((target, axis)) = placement {
            let tab = workspace
                .tabs
                .iter_mut()
                .find(|tab| tab.id == request.tab_id)
                .expect("task tab checked above");
            let old_focus = tab.focused_pane_id;
            tab.focused_pane_id = target;
            let inserted = tab.split_focused(axis, SplitRatio::EQUAL, surface);
            if !focus_task || inserted.is_err() {
                tab.focused_pane_id = old_focus;
            }
            inserted
        } else {
            let old_tab = workspace.active_tab_id;
            let tab = WorkspaceTab::new(&title, surface);
            let pane_id = tab.focused_pane_id;
            let inserted = workspace.add_tab(tab).map(|()| pane_id);
            if !focus_task {
                workspace.active_tab_id = old_tab;
            }
            inserted
        };
        let pane_id = match inserted {
            Ok(pane_id) => pane_id,
            Err(error) => {
                self.report_retained_task(&task, &error.to_string());
                return;
            }
        };
        workspace
            .pane_mut(pane_id)
            .expect("new task pane")
            .task_worktree = Some(task);
        if focus_task {
            self.maximized_pane = None;
        }
        // Durable ownership precedes either launch; a failed launch remains restartable.
        self.sync_session_layout();
        self.status = match self
            .request_terminal_launch(profile, pane_id, title)
            .and_then(|()| self.start_agent_in_pane(request.agent, pane_id))
        {
            Ok(()) => "Task pane created".into(),
            Err(error) => {
                let body = format!("Task pane retained. Restart its terminal to retry: {error}");
                self.global_alerts.push(GlobalAlert {
                    title: "Task agent did not start".into(),
                    body: body.clone(),
                });
                body
            }
        };
    }

    fn report_retained_task(&mut self, task: &TaskWorktree, reason: &str) {
        let body = format!(
            "{reason}. The task worktree is retained at {} (branch {}). Open it through Worktrees to recover it, or remove it there when no longer needed.",
            task.path.display(),
            task.branch
        );
        self.status = body.clone();
        self.global_alerts.push(GlobalAlert {
            title: "Task worktree retained".into(),
            body,
        });
    }

    pub(crate) fn task_removal_busy(&self) -> bool {
        self.task_completion
            .as_ref()
            .is_some_and(|operation| operation.removing)
    }

    pub(crate) fn begin_complete_task(&mut self, pane_id: PaneId) -> Vec<Effect> {
        if self
            .complete_task_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.busy)
        {
            return Vec::new();
        }
        let Some(task) = self.task_worktree(pane_id).cloned() else {
            return Vec::new();
        };
        self.close_command_palette();
        self.pane_menu = None;
        self.active_view = ActiveView::Workspace;
        self.dialog_button = Some(DialogButton::Cancel);
        self.complete_task_prompt = Some(CompleteTaskPrompt {
            pane_id,
            warning: None,
            error: None,
            busy: true,
        });
        let operation = TaskCompletion {
            id: uuid::Uuid::new_v4(),
            epoch: self.task_session_epoch,
            pane_id,
            task: task.clone(),
            removing: false,
        };
        self.task_completion = Some(operation.clone());
        perform_blocking(
            move || crate::task_worktree::inspect(&task),
            move |result| {
                Message::TaskInspected(Box::new((
                    operation,
                    result.and_then(std::convert::identity),
                )))
            },
        )
    }

    fn task_completion_matches(&self, operation: &TaskCompletion) -> bool {
        operation.epoch == self.task_session_epoch
            && self
                .task_completion
                .as_ref()
                .is_some_and(|current| current.id == operation.id)
            && self
                .complete_task_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.pane_id == operation.pane_id)
            && self.task_worktree(operation.pane_id) == Some(&operation.task)
    }

    pub(crate) fn finish_task_inspection(
        &mut self,
        operation: TaskCompletion,
        result: Result<Option<String>, String>,
    ) -> Vec<Effect> {
        if !self.task_completion_matches(&operation) {
            if self
                .task_completion
                .as_ref()
                .is_some_and(|current| current.id == operation.id)
            {
                self.cancel_complete_task();
            }
            return Vec::new();
        }
        let prompt = self.complete_task_prompt.as_mut().expect("matched prompt");
        prompt.busy = false;
        match result {
            Ok(warning) => {
                prompt.warning = warning;
                prompt.error = None;
                if prompt.warning.is_none() {
                    return self.confirm_complete_task();
                }
            }
            Err(error) => prompt.error = Some(error),
        }
        vec![Effect::ScrollToRatio(ScrollTarget::Dialog, 0.0)]
    }

    pub(crate) fn cancel_complete_task(&mut self) {
        if self.task_removal_busy() {
            return;
        }
        self.task_completion = None;
        self.complete_task_prompt = None;
        self.dialog_button = None;
    }

    pub(crate) fn confirm_complete_task(&mut self) -> Vec<Effect> {
        let Some(prompt) = self.complete_task_prompt.as_ref() else {
            return Vec::new();
        };
        if prompt.busy {
            return Vec::new();
        }
        let pane_id = prompt.pane_id;
        // An inspection error can only retry inspection, never grant discard permission.
        if prompt.error.is_some() {
            return self.begin_complete_task(pane_id);
        }
        let discard = prompt.warning.is_some();
        let Some(mut operation) = self.task_completion.clone() else {
            return Vec::new();
        };
        if !self.task_completion_matches(&operation) {
            self.cancel_complete_task();
            return Vec::new();
        }
        if self.terminals.get(&pane_id).is_some_and(|runtime| {
            matches!(
                runtime.launch_state,
                TerminalLaunchState::Starting { .. } | TerminalLaunchState::PreparingHost
            )
        }) {
            self.task_completion_error(
                "The terminal is still starting. Wait for startup to finish, then retry.".into(),
            );
            return Vec::new();
        }
        let references = self.task_directory_references(pane_id);
        if references.iter().any(|path| {
            task_path_contains(&operation.task.path, path, &operation.task.wsl_distribution)
        }) {
            self.task_completion_error("Another pane references this task worktree. Close it or restart it outside the worktree, then retry.".into());
            return Vec::new();
        }
        if let Err(error) = self.ensure_task_replacement(pane_id, &operation.task) {
            self.task_completion_error(error);
            return Vec::new();
        }
        let session = self.terminals.get_mut(&pane_id).and_then(|runtime| {
            runtime.launch_state = TerminalLaunchState::Suppressed;
            runtime.preview =
                "Task terminal stopped. If completion fails, restart the pane to continue working."
                    .into();
            runtime.session.take()
        });
        let stop = Arc::clone(self.task_terminal_stops.entry(pane_id).or_insert_with(|| {
            Arc::new(TaskTerminalStop {
                session: Mutex::new(None),
                pending: AtomicBool::new(true),
            })
        }));
        if let Some(session) = session {
            *stop.session.lock().expect("task terminal stop") = Some(session);
            stop.pending.store(true, Ordering::Release);
        }
        let host = session_host();
        self.queued_terminal_restarts.remove(&pane_id);
        self.clear_pane_activity_state(pane_id);
        operation.removing = true;
        self.task_completion = Some(operation.clone());
        self.complete_task_prompt
            .as_mut()
            .expect("completion prompt")
            .busy = true;
        let task = operation.task.clone();
        perform_blocking(
            move || {
                let mut session = stop
                    .session
                    .lock()
                    .map_err(|_| "The task terminal stop state is unavailable".to_owned())?;
                // Use the captured host, never a later resumed session's daemon.
                if stop.pending.load(Ordering::Acquire) {
                    if let Some((_, client)) = host {
                        client.kill_and_wait(pane_id.as_uuid())?;
                        client.unregister_pane(pane_id.as_uuid());
                    } else if let Some(session) = session.as_ref() {
                        session.terminate_and_wait()?;
                    }
                    // A later Git error must not resurrect this stop obligation:
                    // the daemon has already removed the acknowledged incarnation.
                    stop.pending.store(false, Ordering::Release);
                }
                if let Some(session) = session.take() {
                    dispose_live_session(session);
                }
                drop(session);
                let root = task_resolve_directory(&task.path, &task.wsl_distribution)?;
                for directory in references {
                    let directory = task_resolve_directory(&directory, &task.wsl_distribution)?;
                    if task_path_contains(&root, &directory, &task.wsl_distribution) {
                        return Err("Another pane references this task worktree through a linked path. Restart that pane elsewhere and retry.".into());
                    }
                }
                crate::task_worktree::remove(&task, discard)
            },
            move |result| {
                Message::TaskRemoved(Box::new((
                    operation,
                    result.and_then(std::convert::identity),
                )))
            },
        )
    }

    fn task_completion_error(&mut self, error: String) {
        if let Some(operation) = self.task_completion.as_mut() {
            operation.removing = false;
        }
        if let Some(prompt) = self.complete_task_prompt.as_mut() {
            prompt.busy = false;
            prompt.warning = None;
            prompt.error = Some(error.clone());
        }
        self.dialog_button = Some(DialogButton::Cancel);
        self.status = error;
    }

    pub(crate) fn finish_task_removal(
        &mut self,
        operation: TaskCompletion,
        result: Result<(), String>,
    ) {
        if !self.task_completion_matches(&operation) {
            if let Err(error) = result {
                self.report_retained_task(&operation.task, &error);
            }
            return;
        }
        if let Err(error) = result {
            let stop_pending = self
                .task_terminal_stops
                .get(&operation.pane_id)
                .is_some_and(|stop| stop.pending.load(Ordering::Acquire));
            let guidance = if stop_pending {
                "Task retained; its process has not confirmed stopping. Retry completion before restarting"
            } else {
                "Task retained. Restart the pane to keep working, or retry completion"
            };
            self.task_completion_error(format!("{guidance}: {error}"));
            return;
        }
        self.task_completion = None;
        self.complete_task_prompt = None;
        self.dialog_button = None;
        self.status = match self.close_pane(operation.pane_id) {
            Ok(()) => format!(
                "Task completed; worktree removed. Branch {} retained.",
                operation.task.branch
            ),
            Err(error) => format!("Worktree removed, but its pane could not close: {error}"),
        };
        self.sync_session_layout();
    }

    fn task_directory_references(&self, task_pane: PaneId) -> Vec<PathBuf> {
        let task_backend = self.pane_profile(task_pane).map(|profile| &profile.backend);
        let mut directories = Vec::new();
        for workspace in &self.session.workspaces {
            for tab in &workspace.tabs {
                for (pane_id, pane) in &tab.panes {
                    if *pane_id == task_pane
                        || self.pane_profile(*pane_id).map(|profile| &profile.backend)
                            != task_backend
                    {
                        continue;
                    }
                    if let Some(directory) = self.pane_terminal_directory(*pane_id) {
                        directories.push(directory);
                    }
                    if let Some(directory) = self.pane_working_directory(*pane_id) {
                        directories.push(directory);
                    }
                    if let Some(task) = &pane.task_worktree {
                        directories.push(task.path.clone());
                    }
                    for surface in &pane.surfaces {
                        if let muxtrix_domain::SurfaceKind::Terminal(terminal) = &surface.kind
                            && let Some(directory) = &terminal.working_directory
                        {
                            directories.push(directory.clone());
                        }
                    }
                }
            }
        }
        directories.retain(|directory| reported_path_is_concrete(directory));
        directories.sort();
        directories.dedup();
        directories
    }

    fn ensure_task_replacement(
        &mut self,
        pane_id: PaneId,
        task: &TaskWorktree,
    ) -> Result<(), String> {
        let workspace = self
            .session
            .workspaces
            .iter()
            .find(|workspace| workspace.pane(pane_id).is_some())
            .ok_or("Task workspace is missing")?;
        let tab = workspace
            .tab_containing_pane(pane_id)
            .ok_or("Task tab is missing")?;
        if tab.panes.len() != 1 || workspace.tabs.len() != 1 {
            return Ok(());
        }
        let tab_id = tab.id;
        let mut profile = self.default_terminal_profile()?;
        if profile.working_directory.is_none() {
            profile.working_directory = match &profile.backend {
                ProcessBackend::Local => Some(home_directory().ok_or(
                    "Set a default shell directory before completing the final task pane",
                )?),
                ProcessBackend::Wsl { .. } => Some("~".into()),
            };
        }
        if profile
            .working_directory
            .as_ref()
            .is_some_and(|path| task_path_contains(&task.path, path, &task.wsl_distribution))
        {
            profile.working_directory = Some(task.repo_root.clone());
        }
        let surface = Surface::terminal(
            "shell",
            TerminalSurface {
                profile_id: profile.id,
                working_directory: profile.working_directory.clone(),
            },
        );
        self.clear_manual_layout_history(tab_id);
        let tab = self
            .session
            .workspaces
            .iter_mut()
            .flat_map(|workspace| &mut workspace.tabs)
            .find(|tab| tab.id == tab_id)
            .ok_or("Task tab is missing")?;
        let replacement = tab
            .split_focused(SplitAxis::Horizontal, SplitRatio::EQUAL, surface)
            .map_err(|error| error.to_string())?;
        // Preserve the invariant even if the shell cannot launch: its restart target is safe.
        if let Err(error) = self.request_terminal_launch(profile, replacement, "shell".into()) {
            self.status = format!("Replacement shell needs restart: {error}");
        }
        Ok(())
    }
}

fn task_path_contains(root: &std::path::Path, path: &std::path::Path, distribution: &str) -> bool {
    if cfg!(windows) && distribution.is_empty() && !root.to_string_lossy().starts_with('/') {
        let root = root.to_string_lossy().replace('\\', "/").to_lowercase();
        let path = path.to_string_lossy().replace('\\', "/").to_lowercase();
        std::path::Path::new(&path).starts_with(root)
    } else {
        path.starts_with(root)
    }
}

/// Filesystem resolution belongs on the removal worker, including WSL aliases.
fn task_resolve_directory(path: &std::path::Path, distribution: &str) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    if path.to_string_lossy().starts_with('/') {
        let mut command = wsl_command(distribution);
        command.args(["--exec", "realpath", "-e", "--"]).arg(path);
        let output = command_output(
            &mut command,
            HELPER_COMMAND_TIMEOUT,
            &ProcessCancellation::default(),
        )?;
        if !output.status.success() {
            return Err(format!(
                "Cannot verify another pane's directory {}. Restart that pane in an existing directory and retry.",
                path.display()
            ));
        }
        let resolved = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
        return Ok(PathBuf::from(resolved.trim_end()));
    }
    let _ = distribution;
    std::fs::canonicalize(path).map_err(|error| format!(
        "Cannot verify pane directory {}: {error}. Restart that pane in an existing directory and retry.", path.display()
    ))
}
