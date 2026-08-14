use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

const MANAGED_MARKER: &str = "muxtrix-hook-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Codex,
    Claude,
    Pi,
}

impl Agent {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::Pi];

    const fn slug(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Pi => "pi",
        }
    }

    const fn uses_extension_file(self) -> bool {
        matches!(self, Self::Pi)
    }
}

impl fmt::Display for Agent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.slug())
    }
}

impl FromStr for Agent {
    type Err = HookError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" => Ok(Self::Claude),
            "pi" | "omp" | "oh-my-pi" | "oh_my_pi" => Ok(Self::Pi),
            _ => Err(HookError::UnknownAgent(value.into())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookScope {
    User,
    Project,
}

impl fmt::Display for HookScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::User => "user",
            Self::Project => "project",
        })
    }
}

impl FromStr for HookScope {
    type Err = HookError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "user" | "global" => Ok(Self::User),
            "project" | "local" => Ok(Self::Project),
            _ => Err(HookError::UnknownScope(value.into())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    Add,
    Remove,
    ReAdd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookStatus {
    pub agent: Agent,
    pub scope: HookScope,
    pub target: PathBuf,
    pub installed: bool,
    pub managed_entries: usize,
    pub backup_available: bool,
    /// Managed entries naming an executable that is not on disk.
    ///
    /// A hook is only worth anything if the agent can actually run it, and the
    /// agent reports nothing back when it cannot — the pane simply stops
    /// changing state. Counting these separately is what lets the settings
    /// page say a hook needs repair while it still reads as installed by
    /// every other measure.
    #[serde(default)]
    pub unreachable_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedHookResult {
    pub status: HookStatus,
    pub changed: bool,
    pub message: String,
}

pub struct HookManager {
    home: PathBuf,
    project: PathBuf,
    state_dir: PathBuf,
    executable: PathBuf,
    executable_is_named: bool,
}

impl HookManager {
    pub fn discover(executable: impl Into<PathBuf>) -> Result<Self, HookError> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .ok_or(HookError::HomeMissing)?;
        let project = std::env::current_dir()?;
        let state_dir = hook_state_dir(&home);
        Ok(Self {
            home,
            project,
            state_dir,
            executable: executable.into(),
            executable_is_named: false,
        })
    }

    #[must_use]
    pub fn with_paths(
        home: impl Into<PathBuf>,
        project: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        executable: impl Into<PathBuf>,
    ) -> Self {
        Self {
            home: home.into(),
            project: project.into(),
            state_dir: state_dir.into(),
            executable: executable.into(),
            executable_is_named: false,
        }
    }

    /// Takes the executable on trust rather than checking it is on disk.
    ///
    /// A caller that names a path may mean one this process cannot reach:
    /// installing a WSL distribution's hooks from Windows writes the
    /// distribution's own `/mnt/c` path, which no Windows stat can see. Only a
    /// path Muxtrix derived from its own location is a guess worth checking.
    #[must_use]
    pub fn with_named_executable(mut self) -> Self {
        self.executable_is_named = true;
        self
    }

    #[must_use]
    pub fn project(mut self, project: impl Into<PathBuf>) -> Self {
        self.project = project.into();
        self
    }

    pub fn apply(
        &self,
        agent: Agent,
        scope: HookScope,
        action: HookAction,
    ) -> Result<ManagedHookResult, HookError> {
        // Refuse before touching anything. Writing hooks for an absent
        // executable trades a broken integration for a differently broken one,
        // and doing it inside Re-add would remove the working entries first.
        if matches!(action, HookAction::Add | HookAction::ReAdd) && !self.executable_is_usable() {
            return Err(HookError::ExecutableMissing(self.executable.clone()));
        }
        match action {
            HookAction::Add => self.add(agent, scope),
            HookAction::Remove => self.remove(agent, scope),
            HookAction::ReAdd => {
                let _ = self.remove(agent, scope)?;
                let mut result = self.add(agent, scope)?;
                result.message = format!("Re-added {} {} lifecycle hooks", scope, agent);
                Ok(result)
            }
        }
    }

    pub fn status(&self, agent: Agent, scope: HookScope) -> Result<HookStatus, HookError> {
        if agent.uses_extension_file() {
            return self.extension_status(agent, scope);
        }
        let target = self.target(agent, scope);
        let value = read_json_or_empty(&target)?;
        let managed_entries = count_managed(&value);
        let unreachable_entries = count_unreachable_managed(&value);
        Ok(HookStatus {
            agent,
            scope,
            target,
            // Matching this installation's command text is not enough: a hook
            // whose executable has since been removed still matches, and the
            // agent that fires it gets nothing but a spawn failure. Treat that
            // as needing repair rather than reporting a healthy integration.
            installed: managed_entries == hook_events(agent).len()
                && count_expected_managed(&value, agent, &self.executable) == managed_entries
                && unreachable_entries == 0,
            managed_entries,
            backup_available: self.backup_path(agent, scope).exists(),
            unreachable_entries,
        })
    }

    /// Reads the hook status, first migrating managed hooks whose only
    /// difference from the current installation is the executable path.
    ///
    /// Updating Muxtrix moves the binary without changing hook semantics, and
    /// that alone must never demand a manual repair.
    pub fn synced_status(&self, agent: Agent, scope: HookScope) -> Result<HookStatus, HookError> {
        let status = self.status(agent, scope)?;
        if status.installed || status.managed_entries == 0 {
            return Ok(status);
        }
        // Claiming the user's hooks for an executable that is not on disk
        // replaces a working integration with one that cannot run, silently
        // and at launch. A build shipped without its `muxtrixctl` sibling —
        // a development build of the app alone is the common case — has
        // nothing to migrate to, so it leaves whatever already works in place.
        if !self.executable_is_usable() {
            return Ok(status);
        }
        if agent.uses_extension_file() {
            let target = self.target(agent, scope);
            let text = read_text_or_empty(&target)?;
            if count_semantic_managed_text(&text, agent) != hook_events(agent).len() {
                return Ok(status);
            }
            // Path-only migration must not replace the original backup/record.
            // Otherwise uninstall would restore the stale managed extension the
            // background migration just overwrote.
            write_text_atomic(&target, &managed_extension_source(agent, &self.executable))?;
            return self.status(agent, scope);
        }
        let value = read_json_or_empty(&self.target(agent, scope))?;
        if count_semantic_managed(&value, agent) != hook_events(agent).len() {
            return Ok(status);
        }
        Ok(self.add(agent, scope)?.status)
    }

    /// Whether the `muxtrixctl` this manager would install can be relied on.
    ///
    /// A derived path is a prediction about where the binary sits beside the
    /// running app, and is only worth as much as a look at the disk. A named
    /// one is the caller's business.
    fn executable_is_usable(&self) -> bool {
        self.executable_is_named || self.executable.exists()
    }

    fn add(&self, agent: Agent, scope: HookScope) -> Result<ManagedHookResult, HookError> {
        if agent.uses_extension_file() {
            return self.add_extension(agent, scope);
        }
        let target = self.target(agent, scope);
        let mut value = read_json_or_empty(&target)?;
        let managed_entries = count_managed(&value);
        if managed_entries == hook_events(agent).len()
            && count_expected_managed(&value, agent, &self.executable) == managed_entries
        {
            return Ok(ManagedHookResult {
                status: self.status(agent, scope)?,
                changed: false,
                message: format!("{} {} lifecycle hooks are already installed", scope, agent),
            });
        }

        self.create_backup(agent, scope, &target)?;
        remove_managed(&mut value);
        install_entries(&mut value, agent, &self.executable)?;
        write_json_atomic(&target, &value)?;
        Ok(ManagedHookResult {
            status: self.status(agent, scope)?,
            changed: true,
            message: format!("Added reversible {} {} lifecycle hooks", scope, agent),
        })
    }

    fn remove(&self, agent: Agent, scope: HookScope) -> Result<ManagedHookResult, HookError> {
        if agent.uses_extension_file() {
            return self.remove_extension(agent, scope);
        }
        let target = self.target(agent, scope);
        let existed = target.exists();
        let mut value = read_json_or_empty(&target)?;
        let removed = remove_managed(&mut value);
        let record = self.read_record(agent, scope)?;

        if removed > 0 {
            let should_delete = record
                .as_ref()
                .is_some_and(|record| !record.target_existed && root_is_empty(&value));
            if should_delete {
                if target.exists() {
                    std::fs::remove_file(&target)?;
                }
                remove_empty_parents(&target, agent);
            } else {
                write_json_atomic(&target, &value)?;
            }
        }
        self.remove_backup(agent, scope)?;

        let status = if target.exists() {
            self.status(agent, scope)?
        } else {
            HookStatus {
                agent,
                scope,
                target,
                installed: false,
                managed_entries: 0,
                backup_available: false,
                unreachable_entries: 0,
            }
        };
        Ok(ManagedHookResult {
            status,
            changed: removed > 0 || (!existed && record.is_some()),
            message: if removed > 0 {
                format!(
                    "Removed {} {} lifecycle hooks; unrelated configuration was preserved",
                    scope, agent
                )
            } else {
                format!(
                    "No managed {} {} lifecycle hooks were installed",
                    scope, agent
                )
            },
        })
    }

    fn extension_status(&self, agent: Agent, scope: HookScope) -> Result<HookStatus, HookError> {
        let target = self.target(agent, scope);
        let text = read_text_or_empty(&target)?;
        let managed_entries = count_managed_text(&text, agent);
        let unreachable_entries = if self.executable_is_named {
            0
        } else {
            count_unreachable_managed_text(&text, agent)
        };
        Ok(HookStatus {
            agent,
            scope,
            target,
            installed: managed_entries == hook_events(agent).len()
                && count_expected_managed_text(&text, agent, &self.executable) == managed_entries
                && unreachable_entries == 0,
            managed_entries,
            backup_available: self.backup_path(agent, scope).exists(),
            unreachable_entries,
        })
    }

    fn add_extension(
        &self,
        agent: Agent,
        scope: HookScope,
    ) -> Result<ManagedHookResult, HookError> {
        let target = self.target(agent, scope);
        let text = read_text_or_empty(&target)?;
        let managed_entries = count_managed_text(&text, agent);
        if managed_entries == hook_events(agent).len()
            && count_expected_managed_text(&text, agent, &self.executable) == managed_entries
        {
            return Ok(ManagedHookResult {
                status: self.status(agent, scope)?,
                changed: false,
                message: format!("{} {} lifecycle hooks are already installed", scope, agent),
            });
        }

        self.create_backup(agent, scope, &target)?;
        write_text_atomic(&target, &managed_extension_source(agent, &self.executable))?;
        Ok(ManagedHookResult {
            status: self.status(agent, scope)?,
            changed: true,
            message: format!("Added reversible {} {} lifecycle hooks", scope, agent),
        })
    }

    fn remove_extension(
        &self,
        agent: Agent,
        scope: HookScope,
    ) -> Result<ManagedHookResult, HookError> {
        let target = self.target(agent, scope);
        let existed = target.exists();
        let text = read_text_or_empty(&target)?;
        let removed = count_managed_text(&text, agent);
        let record = self.read_record(agent, scope)?;

        if removed > 0 {
            if let Some(record) = &record {
                if record.target_existed {
                    std::fs::write(&target, std::fs::read(self.backup_path(agent, scope))?)?;
                    set_file_private(&target)?;
                } else if target.exists() {
                    std::fs::remove_file(&target)?;
                    remove_empty_parents(&target, agent);
                }
            } else {
                std::fs::remove_file(&target)?;
                remove_empty_parents(&target, agent);
            }
        }
        self.remove_backup(agent, scope)?;

        let status = if target.exists() {
            self.status(agent, scope)?
        } else {
            HookStatus {
                agent,
                scope,
                target,
                installed: false,
                managed_entries: 0,
                backup_available: false,
                unreachable_entries: 0,
            }
        };
        Ok(ManagedHookResult {
            status,
            changed: removed > 0 || (!existed && record.is_some()),
            message: if removed > 0 {
                format!(
                    "Removed {} {} lifecycle hooks; unrelated configuration was preserved",
                    scope, agent
                )
            } else {
                format!(
                    "No managed {} {} lifecycle hooks were installed",
                    scope, agent
                )
            },
        })
    }

    fn target(&self, agent: Agent, scope: HookScope) -> PathBuf {
        match (agent, scope) {
            (Agent::Codex, HookScope::User) => self.home.join(".codex").join("hooks.json"),
            (Agent::Codex, HookScope::Project) => self.project.join(".codex").join("hooks.json"),
            (Agent::Claude, HookScope::User) => self.home.join(".claude").join("settings.json"),
            (Agent::Claude, HookScope::Project) => {
                self.project.join(".claude").join("settings.local.json")
            }
            (Agent::Pi, HookScope::User) => self
                .home
                .join(".omp")
                .join("agent")
                .join("extensions")
                .join("muxtrix-lifecycle.ts"),
            (Agent::Pi, HookScope::Project) => self
                .project
                .join(".omp")
                .join("extensions")
                .join("muxtrix-lifecycle.ts"),
        }
    }

    fn record_path(&self, agent: Agent, scope: HookScope) -> PathBuf {
        self.state_dir
            .join(format!("{}-{scope}.json", agent.slug()))
    }

    fn backup_path(&self, agent: Agent, scope: HookScope) -> PathBuf {
        self.state_dir
            .join(format!("{}-{scope}.backup", agent.slug()))
    }

    fn create_backup(
        &self,
        agent: Agent,
        scope: HookScope,
        target: &Path,
    ) -> Result<(), HookError> {
        std::fs::create_dir_all(&self.state_dir)?;
        set_directory_private(&self.state_dir)?;
        let target_existed = target.exists();
        let bytes = if target_existed {
            std::fs::read(target)?
        } else {
            Vec::new()
        };
        let backup_path = self.backup_path(agent, scope);
        std::fs::write(&backup_path, bytes)?;
        set_file_private(&backup_path)?;
        let record = BackupRecord {
            target: target.to_path_buf(),
            target_existed,
        };
        write_json_atomic(
            &self.record_path(agent, scope),
            &serde_json::to_value(record)?,
        )
    }

    fn read_record(
        &self,
        agent: Agent,
        scope: HookScope,
    ) -> Result<Option<BackupRecord>, HookError> {
        let path = self.record_path(agent, scope);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
    }

    fn remove_backup(&self, agent: Agent, scope: HookScope) -> Result<(), HookError> {
        for path in [
            self.backup_path(agent, scope),
            self.record_path(agent, scope),
        ] {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        if self.state_dir.exists() && self.state_dir.read_dir()?.next().is_none() {
            std::fs::remove_dir(&self.state_dir)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupRecord {
    target: PathBuf,
    target_existed: bool,
}

fn hook_state_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(base).join("Muxtrix").join("hooks");
    }
    #[cfg(target_os = "macos")]
    return home
        .join("Library")
        .join("Application Support")
        .join("Muxtrix")
        .join("hooks");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_STATE_HOME").map_or_else(
            || {
                home.join(".local")
                    .join("state")
                    .join("muxtrix")
                    .join("hooks")
            },
            |base| PathBuf::from(base).join("muxtrix").join("hooks"),
        )
    }
    #[cfg(target_os = "windows")]
    home.join("AppData")
        .join("Local")
        .join("Muxtrix")
        .join("hooks")
}

fn read_text_or_empty(path: &Path) -> Result<String, HookError> {
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(path).map_err(HookError::Io)
}

fn read_json_or_empty(path: &Path) -> Result<Value, HookError> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if !value.is_object() {
        return Err(HookError::RootNotObject(path.to_path_buf()));
    }
    Ok(value)
}

fn install_entries(root: &mut Value, agent: Agent, executable: &Path) -> Result<(), HookError> {
    let root = root
        .as_object_mut()
        .ok_or_else(|| HookError::RootNotObject(PathBuf::new()))?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(HookError::HooksNotObject)?;

    for &(event, state) in hook_events(agent) {
        let command = hook_command(executable, agent, state);
        let group = json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": 3
            }]
        });
        hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| HookError::EventNotArray(event.into()))?
            .push(group);
    }
    Ok(())
}

/// Installed hook events and the pane state each one reports.
///
/// TeammateIdle and SubagentStop stay installed for stability, but the
/// `hook-event` CLI drops them before they reach the app: helper-agent
/// lifecycle inside a running turn must not repaint the pane. Removing them
/// here instead would change hook semantics and force a manual repair on
/// update.
fn hook_events(agent: Agent) -> &'static [(&'static str, &'static str)] {
    match agent {
        Agent::Codex => &[
            ("SessionStart", "idle"),
            ("UserPromptSubmit", "running"),
            ("PermissionRequest", "waiting"),
            // These wire values preserve the installed hook contract. The app
            // treats PermissionRequest and PostToolUse as advisory metadata:
            // only the live terminal screen may create or clear user attention.
            ("PostToolUse", "running"),
            ("Stop", "completed"),
            ("SessionEnd", "stopped"),
            ("SubagentStart", "running"),
            ("SubagentStop", "completed"),
        ],
        Agent::Claude => &[
            ("SessionStart", "idle"),
            ("UserPromptSubmit", "running"),
            ("Notification", "waiting"),
            ("PermissionRequest", "waiting"),
            ("Stop", "completed"),
            ("SessionEnd", "stopped"),
            ("SubagentStart", "running"),
            ("SubagentStop", "completed"),
            ("TeammateIdle", "waiting"),
        ],
        Agent::Pi => &[
            ("session_start", "idle"),
            ("session_switch", "idle"),
            ("session_branch", "idle"),
            ("agent_start", "running"),
            ("tool_approval_requested", "waiting"),
            ("tool_approval_resolved", "running"),
            ("agent_end", "completed"),
            ("session_shutdown", "stopped"),
        ],
    }
}

fn hook_command(executable: &Path, agent: Agent, state: &str) -> String {
    let executable = executable.to_string_lossy();
    let executable = if cfg!(windows) {
        format!("\"{executable}\"")
    } else {
        format!("'{}'", executable.replace('\'', "'\\''"))
    };
    format!("{executable} {}", hook_command_suffix(agent, state))
}

/// The executable-independent part of a managed hook command. Two commands
/// with the same suffix are semantically identical hooks that may point at
/// different Muxtrix installations.
fn hook_command_suffix(agent: Agent, state: &str) -> String {
    format!(
        "hook-event --managed-by {MANAGED_MARKER} --agent {} --state {state}",
        agent.slug()
    )
}

fn count_managed(root: &Value) -> usize {
    root.get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|handler| is_managed(handler))
        .count()
}

#[cfg(test)]
fn count_managed_for_executable(root: &Value, executable: &Path) -> usize {
    let executable = executable.to_string_lossy();
    root.get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|handler| {
            handler
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| {
                    command.contains(MANAGED_MARKER) && command.contains(executable.as_ref())
                })
        })
        .count()
}

/// Counts managed entries whose executable is not on disk.
///
/// Every managed command starts with its quoted executable, so the path can be
/// read back out of the copy Muxtrix itself wrote. A command that cannot be
/// parsed is left out: an entry Muxtrix does not recognise well enough to read
/// is not one it should call broken.
fn count_unreachable_managed(root: &Value) -> usize {
    let mut checked: BTreeMap<String, bool> = BTreeMap::new();
    root.get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|handler| is_managed(handler))
        .filter_map(|handler| handler.get("command").and_then(Value::as_str))
        .filter_map(managed_executable)
        .filter(|executable| {
            // One stat per distinct path: an agent installs the same
            // executable across every one of its events.
            !*checked
                .entry(executable.clone())
                .or_insert_with(|| Path::new(executable).exists())
        })
        .count()
}

/// Reads the executable back out of a managed hook command.
///
/// The writer quotes it — double quotes on Windows, single quotes elsewhere
/// with `'\''` standing in for an embedded quote — so parsing is the exact
/// inverse of `hook_command`.
fn managed_executable(command: &str) -> Option<String> {
    let mut characters = command.chars();
    let quote = characters.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut executable = String::new();
    let mut rest = characters.as_str();
    loop {
        let (segment, remainder) = rest.split_once(quote)?;
        executable.push_str(segment);
        // `'\''` closes the quote, emits a literal quote, and reopens it.
        if quote == '\''
            && let Some(reopened) = remainder.strip_prefix("\\''")
        {
            executable.push('\'');
            rest = reopened;
            continue;
        }
        return remainder
            .starts_with(char::is_whitespace)
            .then_some(executable);
    }
}

fn count_expected_managed(root: &Value, agent: Agent, executable: &Path) -> usize {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return 0;
    };
    hook_events(agent)
        .iter()
        .filter(|(event, state)| {
            let expected = hook_command(executable, agent, state);
            hooks
                .get(*event)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|group| group.get("hooks").and_then(Value::as_array))
                .flatten()
                .any(|handler| {
                    handler.get("command").and_then(Value::as_str) == Some(expected.as_str())
                })
        })
        .count()
}

/// Counts hook events whose managed command matches the current semantics
/// regardless of which executable it points at.
fn count_semantic_managed(root: &Value, agent: Agent) -> usize {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return 0;
    };
    hook_events(agent)
        .iter()
        .filter(|(event, state)| {
            let suffix = hook_command_suffix(agent, state);
            hooks
                .get(*event)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|group| group.get("hooks").and_then(Value::as_array))
                .flatten()
                .any(|handler| {
                    handler
                        .get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|command| {
                            command.contains(MANAGED_MARKER) && command.ends_with(&suffix)
                        })
                })
        })
        .count()
}

fn count_managed_text(text: &str, agent: Agent) -> usize {
    hook_events(agent)
        .iter()
        .filter(|(event, state)| managed_text_has_event(text, agent, event, state))
        .count()
}

fn count_expected_managed_text(text: &str, agent: Agent, executable: &Path) -> usize {
    let executable = js_string_literal(&executable.to_string_lossy());
    hook_events(agent)
        .iter()
        .filter(|(event, state)| {
            text.contains(&format!("const MUXTRIXCTL = {executable};"))
                && managed_text_has_event(text, agent, event, state)
        })
        .count()
}

fn count_semantic_managed_text(text: &str, agent: Agent) -> usize {
    count_managed_text(text, agent)
}

fn count_unreachable_managed_text(text: &str, agent: Agent) -> usize {
    let Some(executable) = managed_text_executable(text) else {
        return 0;
    };
    if !Path::new(&executable).exists() {
        count_managed_text(text, agent)
    } else {
        0
    }
}

fn managed_text_has_event(text: &str, agent: Agent, event: &str, state: &str) -> bool {
    text.contains(MANAGED_MARKER)
        && text.contains(&format!("agent: \"{}\"", agent.slug()))
        && text.contains(&format!("onLifecycle(\"{event}\", \"{state}\""))
}

fn managed_text_executable(text: &str) -> Option<String> {
    let value = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("const MUXTRIXCTL = ")
            .and_then(|rest| rest.strip_suffix(';'))
    })?;
    serde_json::from_str::<String>(value).ok()
}

fn managed_extension_source(agent: Agent, executable: &Path) -> String {
    let executable = js_string_literal(&executable.to_string_lossy());
    let agent_slug = agent.slug();
    let registrations = if agent == Agent::Pi {
        pi_extension_registrations(agent_slug)
    } else {
        hook_events(agent)
            .iter()
            .map(|(event, state)| {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\");",
                    default_body(agent_slug, state)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"// Managed by Muxtrix ({MANAGED_MARKER}). Remove through `muxtrixctl hooks remove {agent_slug}`.
import {{ spawn }} from "node:child_process";

const MUXTRIXCTL = {executable};
const MANAGED_BY = "{MANAGED_MARKER}";


function sendLifecycle(event, state, message, payload) {{
    const paneId = process.env.MUXTRIX_PANE_ID;
    if (!paneId) return Promise.resolve();
    const body = JSON.stringify({{
        hook_event_name: event,
        agent: "{agent_slug}",
        title: "Oh My Pi",
        message,
        session_id: payload?.sessionId ?? payload?.session_id,
        cwd: process.cwd(),
    }});
    return new Promise((resolve) => {{
        const child = spawn(MUXTRIXCTL, [
            "hook-event",
            "--managed-by",
            MANAGED_BY,
            "--agent",
            "{agent_slug}",
            "--state",
            state,
        ], {{
            env: process.env,
            stdio: ["pipe", "ignore", "ignore"],
            windowsHide: true,
        }});
        child.on("error", resolve);
        child.on("close", resolve);
        child.stdin.end(body);
    }});
}}

function bodyFor(event, payload, fallback) {{
    if (event === "tool_approval_requested" && payload?.toolName) {{
        return `Approval needed: ${{payload.toolName}}`;
    }}
    return fallback;
}}

export default function muxtrixLifecycle(pi) {{
    const pendingApprovals = new Set();
    function onLifecycle(event, state, message, beforeSend) {{
        pi.on(event, async (payload, ctx) => {{
            if (event === "agent_end" && payload?.willContinue) return;
            if (beforeSend && beforeSend(payload) === false) return;
            const body = bodyFor(event, payload, message);
            ctx?.ui?.setStatus?.("muxtrix", `Muxtrix: ${{body}}`);
            await sendLifecycle(event, state, body, payload);
        }});
    }}

{registrations}
}}
"#
    )
}

fn pi_extension_registrations(agent: &str) -> String {
    hook_events(Agent::Pi)
        .iter()
        .map(|(event, state)| match (*event, *state) {
            ("tool_approval_requested", "waiting") => {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\", (payload) => {{
        pendingApprovals.add(payload?.toolCallId ?? \"unknown\");
        return true;
    }});",
                    default_body(agent, state)
                )
            }
            ("tool_approval_resolved", "running") => {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\", (payload) => {{
        pendingApprovals.delete(payload?.toolCallId ?? \"unknown\");
        return pendingApprovals.size === 0;
    }});",
                    default_body(agent, state)
                )
            }
            _ => {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\");",
                    default_body(agent, state)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn default_body(agent: &str, state: &str) -> &'static str {
    let _ = agent;
    match state {
        "idle" => "Ready for input",
        "running" => "Agent is running",
        "waiting" => "Agent needs attention",
        "completed" => "Agent completed a turn",
        "failed" => "Agent reported an error",
        "stopped" => "Agent session ended",
        _ => "Agent lifecycle changed",
    }
}

fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization should not fail")
}

fn remove_managed(root: &mut Value) -> usize {
    let Some(root) = root.as_object_mut() else {
        return 0;
    };
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
    let mut removed = 0;
    hooks.retain(|_, groups| {
        let Some(groups) = groups.as_array_mut() else {
            return true;
        };
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = handlers.len();
            handlers.retain(|handler| !is_managed(handler));
            removed += before - handlers.len();
            !handlers.is_empty()
        });
        !groups.is_empty()
    });
    if hooks.is_empty() {
        root.remove("hooks");
    }
    removed
}

fn is_managed(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains(MANAGED_MARKER))
}

fn root_is_empty(value: &Value) -> bool {
    value.as_object().is_some_and(Map::is_empty)
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<(), HookError> {
    let parent = path
        .parent()
        .ok_or_else(|| HookError::PathHasNoParent(path.to_path_buf()))?;
    std::fs::create_dir_all(parent)?;
    let existing_permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let temporary = path.with_extension("json.muxtrix-tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    if let Some(permissions) = existing_permissions {
        std::fs::set_permissions(&temporary, permissions)?;
    } else {
        set_file_private(&temporary)?;
    }
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(temporary, path)?;
    Ok(())
}
fn write_text_atomic(path: &Path, text: &str) -> Result<(), HookError> {
    let parent = path
        .parent()
        .ok_or_else(|| HookError::PathHasNoParent(path.to_path_buf()))?;
    std::fs::create_dir_all(parent)?;
    let existing_permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let temporary = path.with_extension("ts.muxtrix-tmp");
    std::fs::write(&temporary, text)?;
    if let Some(permissions) = existing_permissions {
        std::fs::set_permissions(&temporary, permissions)?;
    } else {
        set_file_private(&temporary)?;
    }
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(unix)]
fn set_file_private(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_file_private(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(unix)]
fn set_directory_private(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_directory_private(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

fn remove_empty_parents(target: &Path, agent: Agent) {
    if let Some(parent) = target.parent()
        && parent
            .read_dir()
            .is_ok_and(|mut entries| entries.next().is_none())
    {
        let expected = match agent {
            Agent::Codex => ".codex",
            Agent::Claude => ".claude",
            Agent::Pi => "extensions",
        };
        if parent.file_name().is_some_and(|name| name == expected) {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

#[derive(Debug, Error)]
pub enum HookError {
    #[error("home directory could not be discovered")]
    HomeMissing,
    #[error("unknown agent: {0}")]
    UnknownAgent(String),
    #[error("unknown hook scope: {0}")]
    UnknownScope(String),
    #[error("hook configuration root must be an object: {0:?}")]
    RootNotObject(PathBuf),
    #[error("existing hooks value must be an object")]
    HooksNotObject,
    #[error("existing hook event must be an array: {0}")]
    EventNotArray(String),
    #[error("hook path has no parent: {0:?}")]
    PathHasNoParent(PathBuf),
    #[error("muxtrixctl is not at {0:?}, so the hooks it installs could not run")]
    ExecutableMissing(PathBuf),
    #[error("hook file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("hook JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Puts a `muxtrixctl` on disk at `path`.
    ///
    /// Hooks only count as installed when the executable they name is really
    /// there, so a fixture standing for a working installation has to have one.
    fn stub_executable(path: PathBuf) -> PathBuf {
        std::fs::create_dir_all(path.parent().expect("executable should have a parent"))
            .expect("executable directory should be created");
        std::fs::write(&path, b"").expect("executable stub should be written");
        path
    }

    fn fixture() -> (PathBuf, HookManager) {
        let root = std::env::temp_dir().join(format!(
            "muxtrix-hooks-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let manager = HookManager::with_paths(
            root.join("home"),
            root.join("project"),
            root.join("state"),
            stub_executable(root.join("bin").join("muxtrixctl")),
        );
        (root, manager)
    }

    #[test]
    fn session_start_is_idle_until_a_prompt_is_submitted() {
        for agent in [Agent::Codex, Agent::Claude] {
            let events = hook_events(agent);
            assert!(events.contains(&("SessionStart", "idle")));
            assert!(events.contains(&("UserPromptSubmit", "running")));
        }
        let pi_events = hook_events(Agent::Pi);
        assert!(pi_events.contains(&("session_start", "idle")));
        assert!(pi_events.contains(&("session_switch", "idle")));
        assert!(pi_events.contains(&("session_branch", "idle")));
        assert!(pi_events.contains(&("agent_start", "running")));
        assert!(pi_events.contains(&("tool_approval_requested", "waiting")));
        assert!(pi_events.contains(&("tool_approval_resolved", "running")));
    }

    #[test]
    fn codex_permission_hooks_preserve_the_advisory_wire_contract() {
        let events = hook_events(Agent::Codex);
        assert!(events.contains(&("PermissionRequest", "waiting")));
        assert!(events.contains(&("PostToolUse", "running")));
    }

    #[test]
    fn add_remove_and_readd_preserve_unrelated_claude_settings() {
        let (root, manager) = fixture();
        let target = root.join("home/.claude/settings.json");
        std::fs::create_dir_all(target.parent().expect("target should have parent"))
            .expect("fixture directory should exist");
        let original = json!({
            "theme": "dark",
            "hooks": {
                "Stop": [{"hooks": [{"type": "command", "command": "other-tool"}]}]
            }
        });
        std::fs::write(
            &target,
            serde_json::to_vec_pretty(&original).expect("fixture should serialize"),
        )
        .expect("fixture should write");

        let added = manager
            .apply(Agent::Claude, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        assert!(added.changed);
        assert_eq!(added.status.managed_entries, 9);
        assert!(added.status.backup_available);
        let duplicate = manager
            .apply(Agent::Claude, HookScope::User, HookAction::Add)
            .expect("repeated add should work");
        assert!(!duplicate.changed);

        let removed = manager
            .apply(Agent::Claude, HookScope::User, HookAction::Remove)
            .expect("hooks should uninstall");
        assert!(removed.changed);
        assert!(!removed.status.installed);
        assert!(!removed.status.backup_available);
        let restored: Value = serde_json::from_slice(
            &std::fs::read(&target).expect("unrelated settings should remain"),
        )
        .expect("restored settings should parse");
        assert_eq!(restored, original);

        let readded = manager
            .apply(Agent::Claude, HookScope::User, HookAction::ReAdd)
            .expect("hooks should re-add");
        assert!(readded.status.installed);
        assert_eq!(readded.status.managed_entries, 9);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pi_extension_hooks_install_as_autodiscovered_omp_extension() {
        let (root, manager) = fixture();
        let target = root.join("home/.omp/agent/extensions/muxtrix-lifecycle.ts");

        let added = manager
            .apply(Agent::Pi, HookScope::User, HookAction::Add)
            .expect("Pi extension should install");
        assert!(added.changed);
        assert!(added.status.installed);
        assert_eq!(added.status.managed_entries, hook_events(Agent::Pi).len());

        let source = std::fs::read_to_string(&target).expect("extension should exist");
        assert!(source.contains("pi.on(event"));
        assert!(source.contains("agent: \"pi\""));
        assert!(source.contains("muxtrixctl hooks remove pi"));
        assert!(source.contains("payload?.willContinue"));
        assert!(source.contains("pendingApprovals.add"));
        assert!(source.contains("pendingApprovals.size === 0"));
        assert!(source.contains("Approval needed: ${payload.toolName}"));

        let removed = manager
            .apply(Agent::Pi, HookScope::User, HookAction::Remove)
            .expect("Pi extension should remove");
        assert!(removed.changed);
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_deletes_files_and_directories_created_only_for_muxtrix() {
        let (root, manager) = fixture();
        let target = root.join("project/.codex/hooks.json");
        manager
            .apply(Agent::Codex, HookScope::Project, HookAction::Add)
            .expect("hooks should install");
        assert!(target.exists());

        manager
            .apply(Agent::Codex, HookScope::Project, HookAction::Remove)
            .expect("hooks should uninstall");
        assert!(!target.exists());
        assert!(!root.join("project/.codex").exists());
        assert!(!root.join("state").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_preserves_configuration_added_after_muxtrix_installation() {
        let (root, manager) = fixture();
        let target = root.join("home/.codex/hooks.json");
        std::fs::create_dir_all(target.parent().expect("target should have parent"))
            .expect("fixture directory should exist");
        std::fs::write(&target, br#"{"description":"before"}"#).expect("fixture should write");

        manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        let mut changed: Value =
            serde_json::from_slice(&std::fs::read(&target).expect("installed config should exist"))
                .expect("installed config should parse");
        changed["added_later"] = json!(true);
        changed["hooks"]["Stop"]
            .as_array_mut()
            .expect("Stop groups should be an array")
            .push(json!({"hooks": [{"type": "command", "command": "later-tool"}]}));
        std::fs::write(
            &target,
            serde_json::to_vec_pretty(&changed).expect("changed config should serialize"),
        )
        .expect("changed config should write");

        manager
            .apply(Agent::Codex, HookScope::User, HookAction::Remove)
            .expect("hooks should uninstall");
        let cleaned: Value =
            serde_json::from_slice(&std::fs::read(&target).expect("cleaned config should exist"))
                .expect("cleaned config should parse");
        assert_eq!(cleaned["description"], "before");
        assert_eq!(cleaned["added_later"], true);
        assert!(
            cleaned["hooks"]["Stop"]
                .as_array()
                .expect("Stop should remain")
                .iter()
                .any(|group| group["hooks"][0]["command"] == "later-tool")
        );
        assert_eq!(count_managed(&cleaned), 0);
        assert!(!root.join("state").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_existing_json_is_never_overwritten_or_backed_up() {
        let (root, manager) = fixture();
        let target = root.join("home/.claude/settings.json");
        std::fs::create_dir_all(target.parent().expect("target should have parent"))
            .expect("fixture directory should exist");
        let invalid = b"{not-json";
        std::fs::write(&target, invalid).expect("fixture should write");

        assert!(
            manager
                .apply(Agent::Claude, HookScope::User, HookAction::Add)
                .is_err()
        );
        assert_eq!(
            std::fs::read(&target).expect("invalid config should remain"),
            invalid
        );
        assert!(!root.join("state").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn add_replaces_managed_hooks_that_point_to_an_old_executable() {
        let (root, original) = fixture();
        original
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("original hooks should install");
        let replacement_path = stub_executable(root.join("bin").join("muxtrixctl-replacement"));
        let replacement = HookManager::with_paths(
            root.join("home"),
            root.join("project"),
            root.join("replacement-state"),
            &replacement_path,
        );
        assert!(
            !replacement
                .status(Agent::Codex, HookScope::User)
                .expect("status should load")
                .installed
        );

        let result = replacement
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("stale hooks should repair");
        assert!(result.changed);
        assert!(result.status.installed);
        let target = root.join("home/.codex/hooks.json");
        let value: Value =
            serde_json::from_slice(&std::fs::read(target).expect("repaired hooks should exist"))
                .expect("repaired hooks should parse");
        assert_eq!(
            count_managed_for_executable(&value, &replacement_path),
            hook_events(Agent::Codex).len()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn synced_status_migrates_hooks_when_only_the_executable_moved() {
        let (root, original) = fixture();
        original
            .apply(Agent::Claude, HookScope::User, HookAction::Add)
            .expect("original hooks should install");

        let updated_path = stub_executable(root.join("bin-2").join("muxtrixctl"));
        let updated = HookManager::with_paths(
            root.join("home"),
            root.join("project"),
            root.join("state"),
            &updated_path,
        );
        assert!(
            !updated
                .status(Agent::Claude, HookScope::User)
                .expect("status should load")
                .installed
        );

        let synced = updated
            .synced_status(Agent::Claude, HookScope::User)
            .expect("synced status should load");
        assert!(synced.installed, "path-only staleness should self-migrate");

        let value: Value = serde_json::from_slice(
            &std::fs::read(root.join("home/.claude/settings.json"))
                .expect("migrated hooks should exist"),
        )
        .expect("migrated hooks should parse");
        assert_eq!(
            count_expected_managed(&value, Agent::Claude, &updated_path),
            hook_events(Agent::Claude).len()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn synced_status_migrates_pi_extension_without_restoring_stale_uninstall() {
        let (root, original) = fixture();
        original
            .apply(Agent::Pi, HookScope::User, HookAction::Add)
            .expect("original extension should install");

        let target = root.join("home/.omp/agent/extensions/muxtrix-lifecycle.ts");
        let updated_path = stub_executable(root.join("bin-2").join("muxtrixctl"));
        let updated = HookManager::with_paths(
            root.join("home"),
            root.join("project"),
            root.join("state"),
            &updated_path,
        );
        assert!(
            !updated
                .status(Agent::Pi, HookScope::User)
                .expect("status should load")
                .installed
        );

        let synced = updated
            .synced_status(Agent::Pi, HookScope::User)
            .expect("synced status should load");
        assert!(synced.installed, "path-only staleness should self-migrate");
        assert!(
            std::fs::read_to_string(&target)
                .expect("migrated extension should exist")
                .contains(&updated_path.to_string_lossy().to_string())
        );

        let removed = updated
            .apply(Agent::Pi, HookScope::User, HookAction::Remove)
            .expect("migrated extension should remove");
        assert!(removed.changed);
        assert!(!target.exists(), "remove must not restore stale extension");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn hooks_calling_a_deleted_muxtrixctl_ask_for_repair() {
        let (root, manager) = fixture();
        manager
            .apply(Agent::Claude, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        assert!(
            manager
                .status(Agent::Claude, HookScope::User)
                .expect("status should load")
                .installed
        );

        // The binary goes away — an uninstall, a cleaned build directory, a
        // removed worktree. The configuration still names it.
        std::fs::remove_file(root.join("bin").join("muxtrixctl"))
            .expect("executable should be removed");

        let status = manager
            .status(Agent::Claude, HookScope::User)
            .expect("status should load");
        assert!(
            !status.installed,
            "a hook that cannot run must not read as installed"
        );
        assert_eq!(status.managed_entries, hook_events(Agent::Claude).len());
        assert_eq!(status.unreachable_entries, status.managed_entries);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_build_without_muxtrixctl_neither_migrates_nor_installs() {
        let (root, installed) = fixture();
        installed
            .apply(Agent::Claude, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        let working = std::fs::read_to_string(root.join("home/.claude/settings.json"))
            .expect("installed hooks should exist");

        // A development build of the app alone: its muxtrixctl sibling was
        // never produced, so the path it would install is a dead end.
        let barren = HookManager::with_paths(
            root.join("home"),
            root.join("project"),
            root.join("state"),
            root.join("dev-build").join("muxtrixctl"),
        );

        let synced = barren
            .synced_status(Agent::Claude, HookScope::User)
            .expect("synced status should load");
        assert!(!synced.installed);
        assert_eq!(
            std::fs::read_to_string(root.join("home/.claude/settings.json"))
                .expect("hooks should survive"),
            working,
            "a build with no muxtrixctl must leave working hooks untouched"
        );

        // The explicit action refuses too, rather than removing what works and
        // writing something that cannot run in its place.
        let refused = barren.apply(Agent::Claude, HookScope::User, HookAction::ReAdd);
        assert!(matches!(refused, Err(HookError::ExecutableMissing(_))));
        assert_eq!(
            std::fs::read_to_string(root.join("home/.claude/settings.json"))
                .expect("hooks should survive"),
            working
        );

        // A caller that names the path vouches for it; provisioning another
        // environment's hooks must still work.
        assert!(
            barren
                .with_named_executable()
                .apply(Agent::Claude, HookScope::User, HookAction::ReAdd)
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn managed_commands_give_their_executable_back() {
        for executable in [
            Path::new("/mnt/c/Users/user/scoop/apps/muxtrix/current/muxtrixctl.exe"),
            Path::new("/home/user/.muxtrix/bin/muxtrixctl"),
            // A quote inside the path is escaped by the writer, so the reader
            // has to put it back rather than stop at it.
            Path::new("/home/o'brien/bin/muxtrixctl"),
        ] {
            let command = hook_command(executable, Agent::Claude, "completed");
            assert_eq!(
                managed_executable(&command).as_deref(),
                Some(executable.to_string_lossy().as_ref()),
                "could not read the executable back out of {command}"
            );
        }
        // Copy Muxtrix cannot parse is not copy it should call broken.
        assert_eq!(managed_executable("muxtrixctl hook-event"), None);
    }

    #[test]
    fn synced_status_leaves_semantically_changed_hooks_for_manual_repair() {
        let (root, manager) = fixture();
        manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        let target = root.join("home/.codex/hooks.json");
        let mut value: Value =
            serde_json::from_slice(&std::fs::read(&target).expect("installed hooks should exist"))
                .expect("installed hooks should parse");
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("session-start command should exist")
            .replace("--state idle", "--state running");
        value["hooks"]["SessionStart"][0]["hooks"][0]["command"] = json!(command);
        std::fs::write(
            &target,
            serde_json::to_vec_pretty(&value).expect("changed hooks should serialize"),
        )
        .expect("changed hooks should write");
        let before = std::fs::read(&target).expect("changed hooks should read");

        let synced = manager
            .synced_status(Agent::Codex, HookScope::User)
            .expect("synced status should load");
        assert!(!synced.installed);
        assert_eq!(
            std::fs::read(&target).expect("hooks should remain"),
            before,
            "semantic drift must not be rewritten silently"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn synced_status_never_installs_hooks_that_were_never_added() {
        let (root, manager) = fixture();
        let synced = manager
            .synced_status(Agent::Claude, HookScope::User)
            .expect("synced status should load");
        assert!(!synced.installed);
        assert_eq!(synced.managed_entries, 0);
        assert!(!root.join("home/.claude").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn add_repairs_managed_hooks_with_outdated_lifecycle_semantics() {
        let (root, manager) = fixture();
        manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        let target = root.join("home/.codex/hooks.json");
        let mut value: Value =
            serde_json::from_slice(&std::fs::read(&target).expect("installed hooks should exist"))
                .expect("installed hooks should parse");
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("session-start command should exist")
            .replace("--state idle", "--state running");
        value["hooks"]["SessionStart"][0]["hooks"][0]["command"] = json!(command);
        std::fs::write(
            &target,
            serde_json::to_vec_pretty(&value).expect("changed hooks should serialize"),
        )
        .expect("changed hooks should write");

        assert!(
            !manager
                .status(Agent::Codex, HookScope::User)
                .expect("status should load")
                .installed
        );
        let repaired = manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("outdated hooks should repair");
        assert!(repaired.changed);
        assert!(repaired.status.installed);
        let repaired_value: Value =
            serde_json::from_slice(&std::fs::read(&target).expect("repaired hooks should exist"))
                .expect("repaired hooks should parse");
        assert_eq!(
            count_expected_managed(&repaired_value, Agent::Codex, &manager.executable),
            hook_events(Agent::Codex).len()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn existing_configuration_permissions_survive_install_and_remove() {
        use std::os::unix::fs::PermissionsExt as _;

        let (root, manager) = fixture();
        let target = root.join("home/.codex/hooks.json");
        std::fs::create_dir_all(target.parent().expect("target should have parent"))
            .expect("fixture directory should exist");
        std::fs::write(&target, b"{}").expect("fixture should write");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640))
            .expect("permissions should set");

        manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        assert_eq!(
            std::fs::metadata(&target)
                .expect("metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        manager
            .apply(Agent::Codex, HookScope::User, HookAction::Remove)
            .expect("hooks should uninstall");
        assert_eq!(
            std::fs::metadata(&target)
                .expect("metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
