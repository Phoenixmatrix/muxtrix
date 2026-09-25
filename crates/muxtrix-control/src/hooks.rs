use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, value};

const MANAGED_MARKER: &str = "muxtrix-hook-v1";
/// Behavior version of the managed Pi-family extension modules. Bump it when
/// the generated module's semantics change: a module carrying an older number
/// migrates during the next hook synchronization.
const EXTENSION_VERSION: u32 = 6;
const EXTENSION_FILE_NAME: &str = "muxtrix-lifecycle.ts";
const WORKTREE_HOME_FOLDER: &str = ".muxtrix/worktrees";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Codex,
    Claude,
    /// Oh My Pi, the Pi fork that keeps its own `.omp` configuration tree.
    #[serde(rename = "omp", alias = "oh-my-pi")]
    OhMyPi,
    /// Pi itself, configured under `.pi`.
    Pi,
}

impl Agent {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::OhMyPi, Self::Pi];

    const fn slug(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OhMyPi => "omp",
            Self::Pi => "pi",
        }
    }

    /// The name people see for the agent, and the title its managed
    /// extension reports with every lifecycle event.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::OhMyPi => "Oh My Pi",
            Self::Pi => "Pi",
        }
    }

    const fn uses_extension_file(self) -> bool {
        matches!(self, Self::OhMyPi | Self::Pi)
    }

    /// The directory the agent reads its configuration from: under the home
    /// directory (with an `agent` level below it for the Pi family) or the
    /// project root.
    const fn config_dir_name(self) -> &'static str {
        match self {
            Self::Codex => ".codex",
            Self::Claude => ".claude",
            Self::OhMyPi => ".omp",
            Self::Pi => ".pi",
        }
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
            "omp" | "oh-my-pi" | "oh_my_pi" | "ohmypi" => Ok(Self::OhMyPi),
            "pi" | "pi-coding-agent" => Ok(Self::Pi),
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
    worktree_root: PathBuf,
}

impl HookManager {
    pub fn discover(executable: impl Into<PathBuf>) -> Result<Self, HookError> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .ok_or(HookError::HomeMissing)?;
        let project = std::env::current_dir()?;
        let state_dir = hook_state_dir(&home);
        let worktree_root = home.join(WORKTREE_HOME_FOLDER);
        Ok(Self {
            home,
            project,
            state_dir,
            executable: executable.into(),
            executable_is_named: false,
            worktree_root,
        })
    }

    #[must_use]
    pub fn with_paths(
        home: impl Into<PathBuf>,
        project: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        executable: impl Into<PathBuf>,
    ) -> Self {
        let home = home.into();
        Self {
            worktree_root: home.join(WORKTREE_HOME_FOLDER),
            home,
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

    /// Sets the path Codex sees for Muxtrix's shared worktree directory.
    ///
    /// This differs from `home` when a native Windows Muxtrix process edits a
    /// WSL user's configuration over UNC: the file is reached through the UNC
    /// home, while Codex itself must be given the Linux path it runs under.
    #[must_use]
    pub fn worktree_root(mut self, worktree_root: impl Into<PathBuf>) -> Self {
        self.worktree_root = worktree_root.into();
        self
    }

    pub fn apply(
        &self,
        agent: Agent,
        scope: HookScope,
        action: HookAction,
    ) -> Result<ManagedHookResult, HookError> {
        self.adopt_legacy_state(scope)?;
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
        self.adopt_legacy_state(scope)?;
        if agent.uses_extension_file() {
            return self.extension_status(agent, scope);
        }
        let target = self.target(agent, scope);
        let value = read_json_or_empty(&target)?;
        let managed_entries = count_managed(&value);
        // A named executable can belong to another environment. The Windows
        // app, for example, installs `/mnt/c/.../muxtrixctl.exe` into WSL hook
        // files; that path is valid where the hooks run but cannot be statted
        // by Windows. The caller already vouched for named paths, so applying
        // host filesystem reachability checks here would make every successful
        // repair immediately read as broken again.
        let unreachable_entries = if self.executable_is_named {
            0
        } else {
            count_unreachable_managed(&value)
        };
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
        if status.installed {
            if agent == Agent::Codex && scope == HookScope::User {
                self.ensure_codex_worktree_trust()?;
            }
            return Ok(status);
        }
        if status.managed_entries == 0 {
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
            if count_managed_text(&text, agent) != hook_events(agent).len() {
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
        let trust_changed = if agent == Agent::Codex && scope == HookScope::User {
            self.ensure_codex_worktree_trust()?
        } else {
            false
        };
        let managed_entries = count_managed(&value);
        if managed_entries == hook_events(agent).len()
            && count_expected_managed(&value, agent, &self.executable) == managed_entries
        {
            return Ok(ManagedHookResult {
                status: self.status(agent, scope)?,
                changed: trust_changed,
                message: if trust_changed {
                    format!(
                        "{} {} lifecycle hooks are already installed; trusted Muxtrix worktrees",
                        scope, agent
                    )
                } else {
                    format!("{} {} lifecycle hooks are already installed", scope, agent)
                },
            });
        }

        self.create_backup(agent, scope, &target)?;
        remove_managed(&mut value);
        install_entries(&mut value, agent, &self.executable)?;
        write_json_atomic(&target, &value)?;
        Ok(ManagedHookResult {
            status: self.status(agent, scope)?,
            changed: true,
            message: if trust_changed {
                format!(
                    "Added reversible {} {} lifecycle hooks and trusted Muxtrix worktrees",
                    scope, agent
                )
            } else {
                format!("Added reversible {} {} lifecycle hooks", scope, agent)
            },
        })
    }

    /// Trusts the common parent of every worktree Muxtrix creates instead of
    /// making Codex accumulate one project entry per linked checkout.
    fn ensure_codex_worktree_trust(&self) -> Result<bool, HookError> {
        let target = self.home.join(".codex").join("config.toml");
        let contents = match std::fs::read_to_string(&target) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        let mut document = contents.parse::<DocumentMut>()?;
        let worktree_root = self
            .worktree_root
            .to_str()
            .ok_or_else(|| HookError::NonUtf8WorktreeRoot(self.worktree_root.clone()))?;

        let already_trusted = document
            .get("projects")
            .and_then(Item::as_table_like)
            .and_then(|projects| projects.get(worktree_root))
            .and_then(Item::as_table_like)
            .and_then(|project| project.get("trust_level"))
            .and_then(Item::as_str)
            == Some("trusted");
        if already_trusted {
            return Ok(false);
        }

        let projects = document
            .entry("projects")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or_else(|| HookError::CodexProjectsNotTable(target.clone()))?;
        let project = projects
            .entry(worktree_root)
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or_else(|| {
                HookError::CodexProjectNotTable(target.clone(), self.worktree_root.clone())
            })?;
        project.insert("trust_level", value("trusted"));
        write_toml_atomic(&target, &document)?;
        Ok(true)
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
                && extension_version_is_current(&text, agent)
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
            && extension_version_is_current(&text, agent)
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
            (Agent::OhMyPi | Agent::Pi, HookScope::User) => self
                .home
                .join(agent.config_dir_name())
                .join("agent")
                .join("extensions")
                .join(EXTENSION_FILE_NAME),
            (Agent::OhMyPi | Agent::Pi, HookScope::Project) => self
                .project
                .join(agent.config_dir_name())
                .join("extensions")
                .join(EXTENSION_FILE_NAME),
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

    /// Moves an Oh My Pi installation's private record and backup from the
    /// state-file names they had while `pi` was still Oh My Pi's slug.
    ///
    /// Those names now belong to Pi itself. Left in place, Oh My Pi would
    /// report its recovery backup missing and Pi would inherit a record that
    /// describes another agent's file. The record names its target, so the
    /// two are told apart without guessing.
    fn adopt_legacy_state(&self, scope: HookScope) -> Result<(), HookError> {
        let legacy_record = self.state_dir.join(format!("pi-{scope}.json"));
        let record_path = self.record_path(Agent::OhMyPi, scope);
        if record_path.exists() || !legacy_record.exists() {
            return Ok(());
        }
        let record: BackupRecord = serde_json::from_slice(&std::fs::read(&legacy_record)?)?;
        let names_oh_my_pi = record
            .target
            .components()
            .any(|component| component.as_os_str() == Agent::OhMyPi.config_dir_name());
        if !names_oh_my_pi {
            return Ok(());
        }
        std::fs::rename(&legacy_record, &record_path)?;
        let legacy_backup = self.state_dir.join(format!("pi-{scope}.backup"));
        if legacy_backup.exists() {
            std::fs::rename(&legacy_backup, self.backup_path(Agent::OhMyPi, scope))?;
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
        let group = json!({ "hooks": [hook_handler(executable, agent, event, state)] });
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
/// Codex's SubagentStop stays installed for stability, but the `hook-event`
/// CLI drops it before it reaches the app: helper-agent lifecycle inside a
/// running turn must not repaint the pane. Removing it here instead would
/// change hook semantics and force a manual repair on update.
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
        // Claude Code's `--state` is documentary: the `hook-event` client
        // forwards the whole payload and the app decides. The set covers
        // exact turn edges (prompt, stop, failure), the blocking dialogs the
        // harness raises (permission, elicitation, and the notification it
        // sends once a dialog has waited), and session identity.
        Agent::Claude => &[
            ("SessionStart", "idle"),
            ("UserPromptSubmit", "running"),
            ("PermissionRequest", "waiting"),
            ("Elicitation", "waiting"),
            ("Notification", "waiting"),
            ("Stop", "completed"),
            ("StopFailure", "failed"),
            ("SessionEnd", "stopped"),
            ("SubagentStart", "running"),
            ("SubagentStop", "running"),
        ],
        Agent::OhMyPi => &[
            ("session_start", "idle"),
            ("session_switch", "idle"),
            ("session_branch", "idle"),
            ("agent_start", "running"),
            ("tool_approval_requested", "waiting"),
            ("tool_approval_resolved", "running"),
            ("session.compacting", "running"),
            ("session_compact", "running"),
            ("auto_compaction_start", "running"),
            ("auto_compaction_end", "running"),
            ("agent_end", "completed"),
            ("session_shutdown", "stopped"),
        ],
        // Pi's `--state` values are the module's defaults. The generated
        // module tracks the run and prompt edges itself and reports the live
        // state for the events that happen both inside and outside a run.
        // `agent_settled` rather than `agent_end` completes a turn: Pi may
        // still retry, compact, or continue with queued follow-ups after
        // `agent_end`, and `ui_prompt_start`/`ui_prompt_end` bracket every
        // blocking `ctx.ui` prompt an extension raises.
        Agent::Pi => &[
            ("session_start", "idle"),
            ("agent_start", "running"),
            ("ui_prompt_start", "waiting"),
            ("ui_prompt_end", "running"),
            ("session_before_compact", "running"),
            ("session_compact", "running"),
            ("session_compact_failed", "running"),
            ("agent_settled", "completed"),
            ("session_shutdown", "stopped"),
        ],
    }
}

/// The handler object installed for one hook event.
fn hook_handler(executable: &Path, agent: Agent, event: &str, state: &str) -> Value {
    let mut handler = json!({
        "type": "command",
        "command": hook_command(executable, agent, state),
        // Delivery is acknowledged on queue, so this is only ever spent when
        // the app is genuinely unreachable.
        "timeout": 10
    });
    // Launching a Windows executable from WSL intermittently stalls for ten
    // seconds or more inside WSL's interop relay, before `muxtrixctl` even
    // starts. No timeout hides that from a synchronous hook, so Claude Code
    // runs these in the background; the shell stamps each one at fire time
    // so the app can discard an edge that arrives after a newer one.
    // SessionEnd stays synchronous: the harness kills background hooks as it
    // exits.
    if runs_through_wsl_interop(executable, agent) && event != "SessionEnd" {
        handler["async"] = Value::Bool(true);
    }
    handler
}

/// Whether a Claude Code hook would launch a Windows `muxtrixctl` from WSL.
fn runs_through_wsl_interop(executable: &Path, agent: Agent) -> bool {
    let executable = executable.to_string_lossy();
    agent == Agent::Claude
        && executable.starts_with("/mnt/")
        && executable.to_ascii_lowercase().ends_with(".exe")
}

/// Trails a WSL interop command: the fire-time stamp, taken by the hook's
/// own shell rather than by the executable whose launch may stall.
const FIRED_AT_ARGUMENT: &str = r#" --fired-at-ms "$(date +%s%3N)""#;

fn hook_command(executable: &Path, agent: Agent, state: &str) -> String {
    let stamp = if runs_through_wsl_interop(executable, agent) {
        FIRED_AT_ARGUMENT
    } else {
        ""
    };
    let executable = executable.to_string_lossy();
    let executable = if cfg!(windows) {
        format!("\"{executable}\"")
    } else {
        format!("'{}'", executable.replace('\'', "'\\''"))
    };
    format!("{executable} {}{stamp}", hook_command_suffix(agent, state))
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

fn hook_handlers(root: &Value) -> impl Iterator<Item = &Value> {
    root.get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
}

fn event_handlers<'a>(
    hooks: &'a Map<String, Value>,
    event: &str,
) -> impl Iterator<Item = &'a Value> {
    hooks
        .get(event)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
}

fn count_managed(root: &Value) -> usize {
    hook_handlers(root)
        .filter(|handler| is_managed(handler))
        .count()
}

#[cfg(test)]
fn count_managed_for_executable(root: &Value, executable: &Path) -> usize {
    let executable = executable.to_string_lossy();
    hook_handlers(root)
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
    hook_handlers(root)
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
            let expected = hook_handler(executable, agent, event, state);
            event_handlers(hooks, event).any(|handler| {
                handler.get("command") == expected.get("command")
                    && handler.get("async") == expected.get("async")
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
            event_handlers(hooks, event).any(|handler| {
                handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| {
                        let command = command.strip_suffix(FIRED_AT_ARGUMENT).unwrap_or(command);
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

fn extension_version_is_current(text: &str, agent: Agent) -> bool {
    !agent.uses_extension_file()
        || text.contains(&format!(
            "const MUXTRIX_EXTENSION_VERSION = {EXTENSION_VERSION};"
        ))
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
        && managed_text_names_agent(text, agent)
        && text.contains(&format!("onLifecycle(\"{event}\", \"{state}\""))
}

/// Whether a managed module reports as `agent`.
///
/// Modules written while `pi` was Oh My Pi's slug still say `agent: "pi"`.
/// The title they report tells them apart from a Pi module, so they count
/// as Oh My Pi's outdated installation and migrate instead of being
/// abandoned in place while a new module is written beside them.
fn managed_text_names_agent(text: &str, agent: Agent) -> bool {
    let names_slug = |slug: &str| text.contains(&format!("agent: \"{slug}\""));
    let legacy_oh_my_pi =
        names_slug("pi") && text.contains(&format!("title: \"{}\"", Agent::OhMyPi.display_name()));
    match agent {
        Agent::OhMyPi => names_slug(agent.slug()) || legacy_oh_my_pi,
        Agent::Pi => names_slug(agent.slug()) && !legacy_oh_my_pi,
        Agent::Codex | Agent::Claude => names_slug(agent.slug()),
    }
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
    let title = agent.display_name();
    let (body_for, module) = if agent == Agent::Pi {
        (PI_BODY_FOR, pi_extension_module())
    } else {
        (OH_MY_PI_BODY_FOR, oh_my_pi_extension_module())
    };
    format!(
        r#"// Managed by Muxtrix ({MANAGED_MARKER}). Remove through `muxtrixctl hooks remove {agent_slug}`.
import {{ spawn }} from "node:child_process";

const MUXTRIXCTL = {executable};
const MANAGED_BY = "{MANAGED_MARKER}";
const MUXTRIX_EXTENSION_VERSION = {EXTENSION_VERSION};


function sendLifecycle(event, state, message, payload, ctx) {{
    const paneId = process.env.MUXTRIX_PANE_ID;
    if (!paneId) return Promise.resolve();
    let sessionId = payload?.sessionId ?? payload?.session_id;
    if (sessionId === undefined) {{
        try {{
            sessionId = ctx?.sessionManager?.getSessionId?.();
        }} catch {{
            sessionId = undefined;
        }}
    }}
    const body = JSON.stringify({{
        hook_event_name: event,
        agent: "{agent_slug}",
        title: "{title}",
        message,
        session_id: sessionId,
        cwd: ctx?.cwd ?? process.cwd(),
    }});
    return new Promise((resolve) => {{
        try {{
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
            if (!child.stdin) {{
                resolve();
                return;
            }}
            child.stdin.on("error", resolve);
            child.stdin.end(body);
        }} catch {{
            resolve();
        }}
    }});
}}

{body_for}

{module}
"#
    )
}

const OH_MY_PI_BODY_FOR: &str = r#"function bodyFor(event, payload, fallback) {
    if (event === "tool_approval_requested" && payload?.toolName) {
        return `Approval needed: ${payload.toolName}`;
    }
    if (event === "session.compacting") {
        return "Compacting context";
    }
    if (event === "session_compact") {
        return "Context compacted";
    }
    if (event === "auto_compaction_start") {
        if (payload?.action === "handoff") return "Preparing handoff";
        return "Compacting context";
    }
    if (event === "auto_compaction_end") {
        if (payload?.action === "handoff") return "Handoff ready";
        return "Context compacted";
    }
    return fallback;
}"#;

/// Oh My Pi's module: every registered event reports its installed state,
/// except that a continuing `agent_end` stays running and approvals are
/// counted so overlapping requests resolve together.
fn oh_my_pi_extension_module() -> String {
    format!(
        r#"export default function muxtrixLifecycle(pi) {{
    const pendingApprovals = new Set();
    let staleFooterCleared = false;
    function onLifecycle(event, state, message, beforeSend) {{
        pi.on(event, async (payload, ctx) => {{
            if (!staleFooterCleared) {{
                ctx?.ui?.setStatus?.("muxtrix", undefined);
                staleFooterCleared = true;
            }}
            if (event === "agent_end" && payload?.willContinue) {{
                const body = bodyFor(event, payload, "Agent is running");
                await sendLifecycle(event, "running", body, payload);
                return;
            }}
            if (beforeSend && beforeSend(payload) === false) return;
            const body = bodyFor(event, payload, message);
            await sendLifecycle(event, state, body, payload);
        }});
    }}

{registrations}
}}"#,
        registrations = oh_my_pi_extension_registrations()
    )
}

fn oh_my_pi_extension_registrations() -> String {
    hook_events(Agent::OhMyPi)
        .iter()
        .map(|(event, state)| match (*event, *state) {
            ("tool_approval_requested", "waiting") => {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\", (payload) => {{
        pendingApprovals.add(payload?.toolCallId ?? \"unknown\");
        return true;
    }});",
                    default_body(state)
                )
            }
            ("tool_approval_resolved", "running") => {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\", (payload) => {{
        pendingApprovals.delete(payload?.toolCallId ?? \"unknown\");
        return pendingApprovals.size === 0;
    }});",
                    default_body(state)
                )
            }
            ("auto_compaction_end", "running") => {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\", (payload) => !payload?.skipped && !payload?.willRetry);",
                    default_body(state)
                )
            }
            _ => {
                format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\");",
                    default_body(state)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const PI_BODY_FOR: &str = r#"function bodyFor(event, payload, fallback) {
    if (event === "ui_prompt_start" && payload?.title) {
        return `Input needed: ${payload.title}`;
    }
    if (event === "session_before_compact") {
        return "Compacting context";
    }
    if (event === "session_compact") {
        return "Context compacted";
    }
    if (event === "session_compact_failed") {
        return payload?.aborted ? "Compaction cancelled" : "Compaction failed";
    }
    return fallback;
}"#;

/// Pi's module. Each registration's resolver may answer `false` to report
/// nothing or a state string to replace the installed one; the module keeps
/// just enough of Pi's own edges to know whether a run or a prompt is open.
fn pi_extension_module() -> String {
    format!(
        r#"export default function muxtrixLifecycle(pi) {{
    // Pi's events are exact, but compaction and prompts can happen inside or
    // outside an agent run. The run and prompt edges are remembered so those
    // events report the pane's live state instead of a fixed one.
    let agentRunning = false;
    let promptOpen = false;
    let runFailure = null;
    function liveState() {{
        if (promptOpen) return "waiting";
        return agentRunning ? "running" : "idle";
    }}
    function onLifecycle(event, state, message, resolve) {{
        pi.on(event, async (payload, ctx) => {{
            // Print, JSON, and RPC runs have no pane of their own: a `pi -p`
            // started by a tool inside this pane must not repaint it.
            if (ctx?.mode && ctx.mode !== "tui") return;
            const resolved = resolve ? resolve(payload, ctx) : undefined;
            if (resolved === false) return;
            const effective = typeof resolved === "string" ? resolved : state;
            const body = effective === "failed" && runFailure
                ? runFailure
                : bodyFor(event, payload, message);
            await sendLifecycle(event, effective, body, payload, ctx);
        }});
    }}
    // Pi settles without saying how the run ended; the last assistant
    // message of the run does.
    pi.on("message_end", (payload) => {{
        const message = payload?.message;
        if (message?.role !== "assistant" || message.stopReason !== "error") return;
        runFailure = message.errorMessage
            ? `Agent reported an error: ${{message.errorMessage}}`
            : "Agent reported an error";
    }});

{registrations}
}}"#,
        registrations = pi_extension_registrations()
    )
}

fn pi_extension_registrations() -> String {
    hook_events(Agent::Pi)
        .iter()
        .map(|(event, state)| {
            let resolver = match *event {
                // A reload replaces the module while a run may be active.
                "session_start" => Some(
                    "(payload, ctx) => {
        promptOpen = false;
        runFailure = null;
        agentRunning = ctx?.isIdle?.() === false;
        return liveState();
    }",
                ),
                "agent_start" => Some(
                    "() => {
        agentRunning = true;
        runFailure = null;
        return \"running\";
    }",
                ),
                "ui_prompt_start" => Some(
                    "() => {
        promptOpen = true;
        return \"waiting\";
    }",
                ),
                "ui_prompt_end" => Some(
                    "() => {
        promptOpen = false;
        return liveState();
    }",
                ),
                // A manual `/compact` at the prompt is not agent activity.
                "session_before_compact" | "session_compact" | "session_compact_failed" => {
                    Some("() => agentRunning && liveState()")
                }
                "agent_settled" => Some(
                    "(payload, ctx) => {
        // A settled run whose successor is already queued is not done.
        if (ctx?.isIdle?.() === false) return false;
        agentRunning = false;
        promptOpen = false;
        return runFailure ? \"failed\" : \"completed\";
    }",
                ),
                // `/new`, `/resume`, `/fork`, and `/reload` shut the session
                // down and start the next one at once; only quitting stops.
                "session_shutdown" => Some(
                    "(payload) => payload?.reason === undefined || payload?.reason === \"quit\" ? \"stopped\" : false",
                ),
                _ => None,
            };
            match resolver {
                Some(resolver) => format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\", {resolver});",
                    default_body(state)
                ),
                None => format!(
                    "    onLifecycle(\"{event}\", \"{state}\", \"{}\");",
                    default_body(state)
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn default_body(state: &str) -> &'static str {
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
    write_atomic(path, "json.muxtrix-tmp", |temporary| {
        std::fs::write(temporary, serde_json::to_vec_pretty(value)?)?;
        Ok(())
    })
}

fn write_text_atomic(path: &Path, text: &str) -> Result<(), HookError> {
    write_atomic(path, "ts.muxtrix-tmp", |temporary| {
        std::fs::write(temporary, text)?;
        Ok(())
    })
}

fn write_toml_atomic(path: &Path, document: &DocumentMut) -> Result<(), HookError> {
    write_atomic(path, "toml.muxtrix-tmp", |temporary| {
        std::fs::write(temporary, document.to_string())?;
        Ok(())
    })
}

fn write_atomic(
    path: &Path,
    temporary_extension: &str,
    write: impl FnOnce(&Path) -> Result<(), HookError>,
) -> Result<(), HookError> {
    let parent = path
        .parent()
        .ok_or_else(|| HookError::PathHasNoParent(path.to_path_buf()))?;
    std::fs::create_dir_all(parent)?;
    let existing_permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let temporary = path.with_extension(temporary_extension);
    write(&temporary)?;
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
            Agent::Codex | Agent::Claude => agent.config_dir_name(),
            Agent::OhMyPi | Agent::Pi => "extensions",
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
    #[error("Muxtrix's worktree root is not valid UTF-8: {0:?}")]
    NonUtf8WorktreeRoot(PathBuf),
    #[error("Codex projects configuration must be a table: {0:?}")]
    CodexProjectsNotTable(PathBuf),
    #[error("Codex project configuration for {1:?} must be a table: {0:?}")]
    CodexProjectNotTable(PathBuf, PathBuf),
    #[error("muxtrixctl is not at {0:?}, so the hooks it installs could not run")]
    ExecutableMissing(PathBuf),
    #[error("hook file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("hook JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Codex TOML failed: {0}")]
    Toml(#[from] toml_edit::TomlError),
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
        let omp_events = hook_events(Agent::OhMyPi);
        assert!(omp_events.contains(&("session_start", "idle")));
        assert!(omp_events.contains(&("session_switch", "idle")));
        assert!(omp_events.contains(&("session_branch", "idle")));
        assert!(omp_events.contains(&("agent_start", "running")));
        assert!(omp_events.contains(&("tool_approval_requested", "waiting")));
        assert!(omp_events.contains(&("tool_approval_resolved", "running")));
        assert!(omp_events.contains(&("session.compacting", "running")));
        assert!(omp_events.contains(&("session_compact", "running")));
        assert!(omp_events.contains(&("auto_compaction_start", "running")));
        assert!(omp_events.contains(&("auto_compaction_end", "running")));
        let pi_events = hook_events(Agent::Pi);
        assert!(pi_events.contains(&("session_start", "idle")));
        assert!(pi_events.contains(&("agent_start", "running")));
        assert!(pi_events.contains(&("ui_prompt_start", "waiting")));
        assert!(pi_events.contains(&("ui_prompt_end", "running")));
        assert!(pi_events.contains(&("session_before_compact", "running")));
        assert!(pi_events.contains(&("session_compact_failed", "running")));
        // Pi may retry, compact, or continue after `agent_end`; only a
        // settled run is a completed turn.
        assert!(pi_events.contains(&("agent_settled", "completed")));
        assert!(!pi_events.iter().any(|(event, _)| *event == "agent_end"));
        assert!(pi_events.contains(&("session_shutdown", "stopped")));
    }

    #[test]
    fn agent_names_round_trip_between_cli_and_settings() {
        for agent in Agent::ALL {
            assert_eq!(
                Agent::from_str(&agent.to_string()).expect("slug should parse"),
                agent
            );
            let encoded = serde_json::to_string(&agent).expect("agent should serialize");
            assert_eq!(encoded, format!("\"{agent}\""));
            assert_eq!(
                serde_json::from_str::<Agent>(&encoded).expect("agent should deserialize"),
                agent
            );
        }
        assert!(matches!(Agent::from_str("oh-my-pi"), Ok(Agent::OhMyPi)));
        assert!(matches!(Agent::from_str("OMP"), Ok(Agent::OhMyPi)));
        assert!(matches!(Agent::from_str("pi"), Ok(Agent::Pi)));
        assert!(Agent::from_str("tau").is_err());
        assert_eq!(Agent::OhMyPi.to_string(), "omp");
        assert_eq!(Agent::Pi.to_string(), "pi");
        assert_eq!(Agent::OhMyPi.display_name(), "Oh My Pi");
        assert_eq!(Agent::Pi.display_name(), "Pi");
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
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oh_my_pi_extension_hooks_install_as_autodiscovered_omp_extension() {
        let (root, manager) = fixture();
        let target = root.join("home/.omp/agent/extensions/muxtrix-lifecycle.ts");

        let added = manager
            .apply(Agent::OhMyPi, HookScope::User, HookAction::Add)
            .expect("Oh My Pi extension should install");
        assert!(added.changed);
        assert!(added.status.installed);
        assert_eq!(
            added.status.managed_entries,
            hook_events(Agent::OhMyPi).len()
        );

        let source = std::fs::read_to_string(&target).expect("extension should exist");
        assert!(source.contains("pi.on(event"));
        assert!(source.contains("agent: \"omp\""));
        assert!(source.contains("title: \"Oh My Pi\""));
        assert!(source.contains("muxtrixctl hooks remove omp"));
        assert!(source.contains("payload?.willContinue"));
        assert!(source.contains("pendingApprovals.add"));
        assert!(source.contains("pendingApprovals.size === 0"));
        assert!(source.contains("Approval needed: ${payload.toolName}"));
        assert!(source.contains("Preparing handoff"));
        assert!(source.contains("onLifecycle(\"auto_compaction_start\", \"running\""));
        assert!(source.contains("onLifecycle(\"session_compact\", \"running\""));
        assert!(source.contains("onLifecycle(\"auto_compaction_end\", \"running\""));
        assert!(source.contains("!payload?.skipped && !payload?.willRetry"));
        assert!(source.contains("sendLifecycle(event, \"running\""));
        assert!(source.contains("setStatus?.(\"muxtrix\", undefined)"));
        assert!(!source.contains("`Muxtrix: ${body}`"));
        assert!(source.contains(&format!(
            "const MUXTRIX_EXTENSION_VERSION = {EXTENSION_VERSION};"
        )));
        // A broken WSL interop handler makes Bun throw synchronously on
        // spawn and the pipe fail on write; lifecycle reporting is
        // best-effort and must resolve through every one of those paths.
        assert!(source.contains("child.on(\"error\", resolve);"));
        assert!(source.contains("if (!child.stdin) {"));
        assert!(source.contains("resolve();\n                return;"));
        assert!(
            source.contains(
                "child.stdin.on(\"error\", resolve);\n            child.stdin.end(body);"
            )
        );
        assert!(source.contains("} catch {\n            resolve();\n        }"));

        let removed = manager
            .apply(Agent::OhMyPi, HookScope::User, HookAction::Remove)
            .expect("Oh My Pi extension should remove");
        assert!(removed.changed);
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pi_extension_hooks_install_as_autodiscovered_pi_extension() {
        let (root, manager) = fixture();
        let target = root.join("home/.pi/agent/extensions/muxtrix-lifecycle.ts");

        let added = manager
            .apply(Agent::Pi, HookScope::User, HookAction::Add)
            .expect("Pi extension should install");
        assert!(added.changed);
        assert!(added.status.installed);
        assert_eq!(added.status.managed_entries, hook_events(Agent::Pi).len());
        assert_eq!(added.status.target, target);

        let source = std::fs::read_to_string(&target).expect("extension should exist");
        assert!(source.contains("agent: \"pi\""));
        assert!(source.contains("title: \"Pi\""));
        assert!(source.contains("muxtrixctl hooks remove pi"));
        assert!(source.contains(&format!(
            "const MUXTRIX_EXTENSION_VERSION = {EXTENSION_VERSION};"
        )));
        // Pi's lifecycle is reported from its own events alone.
        assert!(source.contains("onLifecycle(\"agent_start\", \"running\""));
        assert!(source.contains("onLifecycle(\"ui_prompt_start\", \"waiting\""));
        assert!(source.contains("onLifecycle(\"ui_prompt_end\", \"running\""));
        assert!(source.contains("onLifecycle(\"agent_settled\", \"completed\""));
        assert!(source.contains("onLifecycle(\"session_shutdown\", \"stopped\""));
        assert!(!source.contains("onLifecycle(\"agent_end\""));
        assert!(source.contains("if (ctx?.isIdle?.() === false) return false;"));
        assert!(source.contains("agentRunning = ctx?.isIdle?.() === false;"));
        assert!(source.contains("() => agentRunning && liveState()"));
        assert!(source.contains("payload?.reason === \"quit\" ? \"stopped\" : false"));
        assert!(source.contains("if (ctx?.mode && ctx.mode !== \"tui\") return;"));
        assert!(source.contains("message.stopReason !== \"error\""));
        assert!(source.contains("ctx?.sessionManager?.getSessionId?.()"));
        assert!(source.contains("cwd: ctx?.cwd ?? process.cwd()"));
        assert!(source.contains("Input needed: ${payload.title}"));
        // Pi's own approval vocabulary never appears in this module.
        assert!(!source.contains("tool_approval_requested"));
        assert!(!source.contains("willContinue"));

        let project_target = root.join("project/.pi/extensions/muxtrix-lifecycle.ts");
        let project = manager
            .apply(Agent::Pi, HookScope::Project, HookAction::Add)
            .expect("project Pi extension should install");
        assert!(project.status.installed);
        assert_eq!(project.status.target, project_target);

        let removed = manager
            .apply(Agent::Pi, HookScope::User, HookAction::Remove)
            .expect("Pi extension should remove");
        assert!(removed.changed);
        assert!(!target.exists());
        assert!(
            !target
                .parent()
                .expect("target should have a parent")
                .exists(),
            "an extensions directory Muxtrix created is removed with its module"
        );
        assert!(
            manager
                .apply(Agent::Pi, HookScope::Project, HookAction::Remove)
                .expect("project Pi extension should remove")
                .changed
        );
        assert!(!project_target.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A module written while `pi` was Oh My Pi's slug: the same events, the
    /// old agent slug, and the previous behavior version.
    fn legacy_oh_my_pi_module(manager: &HookManager) -> String {
        managed_extension_source(Agent::OhMyPi, &manager.executable)
            .replace("agent: \"omp\"", "agent: \"pi\"")
            .replace(
                "\"omp\",\n                \"--state\"",
                "\"pi\",\n                \"--state\"",
            )
            .replace(
                &format!("const MUXTRIX_EXTENSION_VERSION = {EXTENSION_VERSION};"),
                "const MUXTRIX_EXTENSION_VERSION = 5;",
            )
    }

    #[test]
    fn legacy_oh_my_pi_module_migrates_instead_of_being_abandoned() {
        let (root, manager) = fixture();
        let target = root.join("home/.omp/agent/extensions/muxtrix-lifecycle.ts");
        std::fs::create_dir_all(target.parent().expect("target should have a parent"))
            .expect("extension directory should be created");
        let legacy = legacy_oh_my_pi_module(&manager);
        assert!(legacy.contains("agent: \"pi\""));
        std::fs::write(&target, &legacy).expect("legacy module should be written");

        // It is Oh My Pi's outdated installation, not Pi's.
        assert_eq!(
            count_managed_text(&legacy, Agent::OhMyPi),
            hook_events(Agent::OhMyPi).len()
        );
        assert_eq!(count_managed_text(&legacy, Agent::Pi), 0);
        let status = manager
            .status(Agent::OhMyPi, HookScope::User)
            .expect("status should load");
        assert!(!status.installed);
        assert_eq!(status.managed_entries, hook_events(Agent::OhMyPi).len());
        assert!(
            !manager
                .status(Agent::Pi, HookScope::User)
                .expect("Pi status should load")
                .installed
        );

        let synced = manager
            .synced_status(Agent::OhMyPi, HookScope::User)
            .expect("legacy module should migrate");
        assert!(synced.installed);
        let migrated = std::fs::read_to_string(&target).expect("migrated module should exist");
        assert!(migrated.contains("agent: \"omp\""));
        assert!(!migrated.contains("agent: \"pi\""));
        assert!(extension_version_is_current(&migrated, Agent::OhMyPi));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oh_my_pi_recovery_state_follows_its_new_name() {
        let (root, manager) = fixture();
        let omp_target = root.join("home/.omp/agent/extensions/muxtrix-lifecycle.ts");
        std::fs::create_dir_all(&manager.state_dir).expect("state directory should be created");
        // The record and backup an older build wrote for Oh My Pi under the
        // `pi` name, alongside a record Pi wrote for itself.
        std::fs::write(
            manager.state_dir.join("pi-user.json"),
            serde_json::to_vec(&BackupRecord {
                target: omp_target.clone(),
                target_existed: true,
            })
            .expect("record should serialize"),
        )
        .expect("legacy record should be written");
        std::fs::write(
            manager.state_dir.join("pi-user.backup"),
            b"original omp module",
        )
        .expect("legacy backup should be written");
        let pi_record = BackupRecord {
            target: root.join("project/.pi/extensions/muxtrix-lifecycle.ts"),
            target_existed: false,
        };
        std::fs::write(
            manager.state_dir.join("pi-project.json"),
            serde_json::to_vec(&pi_record).expect("record should serialize"),
        )
        .expect("Pi record should be written");

        // Any status read adopts the misnamed files.
        let pi_status = manager
            .status(Agent::Pi, HookScope::User)
            .expect("Pi status should load");
        assert!(!pi_status.backup_available);
        assert!(!manager.state_dir.join("pi-user.json").exists());
        assert!(!manager.state_dir.join("pi-user.backup").exists());
        assert!(manager.state_dir.join("omp-user.json").exists());
        assert_eq!(
            std::fs::read(manager.state_dir.join("omp-user.backup"))
                .expect("adopted backup should exist"),
            b"original omp module"
        );
        assert!(
            manager
                .status(Agent::OhMyPi, HookScope::User)
                .expect("Oh My Pi status should load")
                .backup_available
        );
        // A record that names Pi's own file is left where it is.
        assert!(
            manager
                .status(Agent::Pi, HookScope::Project)
                .expect("Pi project status should load")
                .backup_available
                == manager.state_dir.join("pi-project.backup").exists()
        );
        assert!(manager.state_dir.join("pi-project.json").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_user_hooks_trust_the_shared_worktree_parent_without_reformatting_config() {
        let (root, manager) = fixture();
        let target = root.join("home/.codex/config.toml");
        std::fs::create_dir_all(target.parent().expect("config should have a parent"))
            .expect("fixture directory should exist");
        std::fs::write(
            &target,
            "# keep this comment\nmodel = \"gpt-5.6\"\n\n[projects.\"/tmp/other\"]\ntrust_level = \"untrusted\"\n",
        )
        .expect("fixture config should write");

        let added = manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("hooks and worktree trust should install");
        assert!(added.changed);
        assert!(added.message.contains("trusted Muxtrix worktrees"));

        let contents = std::fs::read_to_string(&target).expect("Codex config should exist");
        assert!(contents.contains("# keep this comment"));
        let document = contents
            .parse::<DocumentMut>()
            .expect("Codex config should remain valid TOML");
        let projects = document["projects"]
            .as_table_like()
            .expect("projects should remain a table");
        assert_eq!(
            projects
                .get("/tmp/other")
                .and_then(Item::as_table_like)
                .and_then(|project| project.get("trust_level"))
                .and_then(Item::as_str),
            Some("untrusted")
        );
        let worktree_root = root.join("home/.muxtrix/worktrees");
        assert_eq!(
            projects
                .get(worktree_root.to_string_lossy().as_ref())
                .and_then(Item::as_table_like)
                .and_then(|project| project.get("trust_level"))
                .and_then(Item::as_str),
            Some("trusted")
        );

        let before = std::fs::read(&target).expect("Codex config should read");
        let duplicate = manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("repeated setup should work");
        assert!(!duplicate.changed);
        assert_eq!(
            std::fs::read(&target).expect("Codex config should still read"),
            before,
            "idempotent setup must not rewrite the config"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn existing_codex_hooks_gain_worktree_trust_during_synced_discovery() {
        let (root, manager) = fixture();
        manager
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("hooks should install");
        let config = root.join("home/.codex/config.toml");
        std::fs::remove_file(&config).expect("trust config should be removed for the fixture");

        let status = manager
            .synced_status(Agent::Codex, HookScope::User)
            .expect("existing hooks should migrate trust");
        assert!(status.installed);
        let contents = std::fs::read_to_string(config).expect("trust config should be restored");
        assert!(contents.contains("trust_level = \"trusted\""));
        assert!(
            contents.contains(
                root.join("home/.muxtrix/worktrees")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_trust_uses_the_agent_visible_worktree_path() {
        let (root, manager) = fixture();
        let visible_root = Path::new("/home/user/.muxtrix/worktrees");
        manager
            .worktree_root(visible_root)
            .apply(Agent::Codex, HookScope::User, HookAction::Add)
            .expect("hooks and WSL-visible trust should install");

        let contents = std::fs::read_to_string(root.join("home/.codex/config.toml"))
            .expect("Codex config should exist");
        let document = contents
            .parse::<DocumentMut>()
            .expect("Codex config should be valid");
        assert_eq!(
            document["projects"][visible_root.to_string_lossy().as_ref()]["trust_level"].as_str(),
            Some("trusted")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_codex_toml_is_never_overwritten_and_prevents_hook_installation() {
        let (root, manager) = fixture();
        let config = root.join("home/.codex/config.toml");
        std::fs::create_dir_all(config.parent().expect("config should have a parent"))
            .expect("fixture directory should exist");
        let invalid = b"[projects\n";
        std::fs::write(&config, invalid).expect("invalid fixture should write");

        assert!(matches!(
            manager.apply(Agent::Codex, HookScope::User, HookAction::Add),
            Err(HookError::Toml(_))
        ));
        assert_eq!(
            std::fs::read(config).expect("invalid config should remain"),
            invalid
        );
        assert!(!root.join("home/.codex/hooks.json").exists());
        assert!(!root.join("state").exists());
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
    fn wsl_interop_claude_hooks_run_in_the_background_and_migrate_from_synchronous() {
        let (root, _) = fixture();
        let interop = Path::new("/mnt/c/Users/user/scoop/apps/muxtrix/current/muxtrixctl.exe");
        let manager = HookManager::with_paths(
            root.join("home"),
            root.join("project"),
            root.join("state"),
            interop,
        )
        .with_named_executable();
        let target = root.join("home/.claude/settings.json");

        // The synchronous hooks earlier releases installed.
        let mut value = json!({});
        {
            let hooks = value
                .as_object_mut()
                .expect("root is an object")
                .entry("hooks")
                .or_insert_with(|| json!({}));
            for &(event, state) in hook_events(Agent::Claude) {
                hooks[event] = json!([{ "hooks": [{
                    "type": "command",
                    "command": format!(
                        "\"{}\" {}",
                        interop.display(),
                        hook_command_suffix(Agent::Claude, state)
                    ),
                    "timeout": 10
                }]}]);
            }
        }
        std::fs::create_dir_all(target.parent().expect("settings has a parent"))
            .expect("settings directory");
        std::fs::write(
            &target,
            serde_json::to_vec(&value).expect("hooks serialize"),
        )
        .expect("old hooks written");

        let synced = manager
            .synced_status(Agent::Claude, HookScope::User)
            .expect("synced status should load");
        assert!(
            synced.installed,
            "synchronous interop hooks should self-migrate"
        );

        let value: Value = serde_json::from_slice(&std::fs::read(&target).expect("hooks exist"))
            .expect("hooks parse");
        let hooks = value["hooks"].as_object().expect("hooks object");
        for &(event, _) in hook_events(Agent::Claude) {
            let handler = event_handlers(hooks, event)
                .next()
                .expect("handler installed");
            let command = handler["command"].as_str().expect("command");
            assert!(command.ends_with(FIRED_AT_ARGUMENT), "{event}: {command}");
            // The harness kills background hooks as it exits.
            assert_eq!(
                handler.get("async").and_then(Value::as_bool),
                (event != "SessionEnd").then_some(true),
                "{event}"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_hooks_stay_synchronous_and_unstamped() {
        let native = Path::new("/home/user/.muxtrix/bin/muxtrixctl");
        let windows_codex = Path::new("/mnt/c/Users/user/muxtrixctl.exe");
        for (executable, agent) in [(native, Agent::Claude), (windows_codex, Agent::Codex)] {
            let handler = hook_handler(executable, agent, "Stop", "completed");
            assert_eq!(handler.get("async"), None);
            assert!(
                !handler["command"]
                    .as_str()
                    .expect("command")
                    .contains("--fired-at-ms")
            );
        }
    }

    #[test]
    fn synced_status_migrates_oh_my_pi_extension_without_restoring_stale_uninstall() {
        let (root, original) = fixture();
        original
            .apply(Agent::OhMyPi, HookScope::User, HookAction::Add)
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
                .status(Agent::OhMyPi, HookScope::User)
                .expect("status should load")
                .installed
        );

        let synced = updated
            .synced_status(Agent::OhMyPi, HookScope::User)
            .expect("synced status should load");
        assert!(synced.installed, "path-only staleness should self-migrate");
        assert!(
            std::fs::read_to_string(&target)
                .expect("migrated extension should exist")
                .contains(&updated_path.to_string_lossy().to_string())
        );

        let removed = updated
            .apply(Agent::OhMyPi, HookScope::User, HookAction::Remove)
            .expect("migrated extension should remove");
        assert!(removed.changed);
        assert!(!target.exists(), "remove must not restore stale extension");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn synced_status_migrates_outdated_oh_my_pi_extension_behavior() {
        let (root, manager) = fixture();
        manager
            .apply(Agent::OhMyPi, HookScope::User, HookAction::Add)
            .expect("current extension should install");

        let target = root.join("home/.omp/agent/extensions/muxtrix-lifecycle.ts");
        let current = std::fs::read_to_string(&target).expect("extension should exist");
        let stale = current
            .replace(
                &format!("const MUXTRIX_EXTENSION_VERSION = {EXTENSION_VERSION};"),
                "const MUXTRIX_EXTENSION_VERSION = 4;",
            )
            .replace(
                "await sendLifecycle(event, state, body, payload);",
                "ctx?.ui?.setStatus?.(\"muxtrix\", `Muxtrix: ${body}`);\n            await sendLifecycle(event, state, body, payload);",
            );
        std::fs::write(&target, stale).expect("stale extension should be written");
        assert!(
            !manager
                .status(Agent::OhMyPi, HookScope::User)
                .expect("status should load")
                .installed
        );

        let synced = manager
            .synced_status(Agent::OhMyPi, HookScope::User)
            .expect("outdated extension should migrate");
        assert!(synced.installed);
        let migrated = std::fs::read_to_string(&target).expect("migrated extension should exist");
        assert!(extension_version_is_current(&migrated, Agent::OhMyPi));
        assert!(migrated.contains("setStatus?.(\"muxtrix\", undefined)"));
        assert!(!migrated.contains("`Muxtrix: ${body}`"));
        assert!(!migrated.contains("const MUXTRIX_EXTENSION_VERSION = 4;"));
        assert!(
            migrated.contains(
                "child.stdin.on(\"error\", resolve);\n            child.stdin.end(body);"
            )
        );
        assert!(migrated.contains("} catch {\n            resolve();\n        }"));
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
        let externally_named = barren
            .with_named_executable()
            .apply(Agent::Claude, HookScope::User, HookAction::ReAdd)
            .expect("a named cross-environment executable should install");
        assert!(
            externally_named.status.installed,
            "a path the caller vouched for must stay installed after repair"
        );
        assert_eq!(externally_named.status.unreachable_entries, 0);
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
        let config = root.join("home/.codex/config.toml");
        std::fs::create_dir_all(target.parent().expect("target should have parent"))
            .expect("fixture directory should exist");
        std::fs::write(&target, b"{}").expect("fixture should write");
        std::fs::write(&config, b"").expect("Codex config fixture should write");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640))
            .expect("permissions should set");
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640))
            .expect("Codex config permissions should set");

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
        assert_eq!(
            std::fs::metadata(&config)
                .expect("Codex config should exist")
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
        assert_eq!(
            std::fs::metadata(&config)
                .expect("Codex config should remain")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
