//! Private, bounded Claude lifecycle spool. Immutable files are published only
//! after syncing; an OS file lock serializes hook processes and checkpoints.
//! Checkpoints carry acknowledgements so a crash during cleanup cannot replay
//! already-reduced events. Conversation text is never persisted here.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::ClaudeHook;

const MAX_EVENTS: usize = 512;
const MAX_EVENT_BYTES: u64 = 256 * 1024;
const MAX_CHECKPOINT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = MAX_EVENTS + 16;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub struct ClaudeJournal {
    directory: PathBuf,
    fingerprint: Option<Vec<(String, u64, Option<std::time::SystemTime>)>>,
}

#[derive(Debug, Default)]
pub struct JournalReplay {
    pub checkpoint: Option<serde_json::Value>,
    pub events: Vec<ClaudeHook>,
    pub incomplete: bool,
    /// Opaque identities of exactly the uncertainty observed by this load.
    pub gaps: Vec<String>,
    /// Latest causal boundary of the observed gaps. Only authoritative inventory
    /// strictly newer than this boundary can supersede their uncertainty.
    /// Zero with `incomplete` means the boundary could not be ordered.
    pub gap_at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    state: serde_json::Value,
    acknowledged: Vec<String>,
    #[serde(default)]
    acknowledged_gaps: Vec<String>,
}

impl ClaudeJournal {
    pub fn for_pane(pane: &str) -> io::Result<Self> {
        Self::in_directory(&crate::transport::control_registry_directory(), pane)
    }

    /// Opens a journal under an explicit control registry root, allowing
    /// isolated recovery tests without process-global environment mutation.
    pub fn in_directory(root: &Path, pane: &str) -> io::Result<Self> {
        if pane.is_empty() || pane.len() > 96 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid pane identity length",
            ));
        }
        // Reversible hex encoding has neither separators nor hash collisions.
        let name: String = pane
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        private_directory(root)?;
        let base = root.join("claude-activity");
        private_directory(&base)?;
        let directory = base.join(name);
        private_directory(&directory)?;
        Ok(Self {
            directory,
            fingerprint: None,
        })
    }

    fn lock(&self) -> io::Result<File> {
        let file = private_options()
            .create(true)
            .read(true)
            .write(true)
            .open(self.directory.join("lock"))?;
        file.lock()?;
        Ok(file)
    }

    fn entries(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.directory)?.take(MAX_ENTRIES + 1) {
            paths.push(entry?.path());
        }
        if paths.len() > MAX_ENTRIES {
            return Err(io::Error::other("Claude journal directory limit exceeded"));
        }
        paths.sort();
        Ok(paths)
    }

    /// Metadata-only change detection; no hook or checkpoint bodies are read.
    pub fn changed(&mut self) -> io::Result<bool> {
        let _lock = self.lock()?;
        let mut fingerprint = Vec::new();
        for path in self.entries()? {
            let metadata = fs::symlink_metadata(&path)?;
            fingerprint.push((
                path.file_name()
                    .expect("journal entry has a file name")
                    .to_string_lossy()
                    .into_owned(),
                metadata.len(),
                metadata.modified().ok(),
            ));
        }
        let changed = self.fingerprint.as_ref() != Some(&fingerprint);
        self.fingerprint = Some(fingerprint);
        Ok(changed)
    }

    fn mark_incomplete(&self, at_ms: u64) -> io::Result<()> {
        // Coalesce repeated failures into a fresh immutable identity. An older
        // replay can never acknowledge this replacement by accident.
        let old: Vec<_> = fs::read_dir(&self.directory)?
            .take(MAX_ENTRIES + 1)
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| extension(path, "gap"))
            .collect();
        let name = format!("{}.gap", unique_id());
        let mut boundary = if at_ms == 0 {
            publication_time_ms(Path::new(&name))
        } else {
            at_ms
        };
        for path in &old {
            boundary = merge_gap_time(boundary, gap_time_ms(path));
        }
        self.publish(
            &name,
            &serde_json::to_vec(&boundary).map_err(io::Error::other)?,
        )?;
        for path in old {
            remove_if_exists(&path)?;
        }
        sync_directory(&self.directory)
    }

    /// Returns only after the event is durable. The caller must invoke this
    /// before sending IPC; GUI availability is irrelevant to persistence.
    pub fn append(&self, hook: &mut ClaudeHook) -> io::Result<()> {
        let _lock = self.lock()?;
        let paths = self.entries()?;
        if paths.iter().filter(|path| extension(path, "event")).count() >= MAX_EVENTS {
            self.mark_incomplete(hook.sent_at_ms)?;
            return Err(io::Error::other("Claude journal event limit exceeded"));
        }
        let id = unique_id();
        hook.delivery_id = Some(id.clone());
        let mut durable = hook.clone();
        durable.message = None;
        durable.last_assistant_message = None;
        let bytes = serde_json::to_vec(&durable).map_err(io::Error::other)?;
        if bytes.len() as u64 > MAX_EVENT_BYTES {
            self.mark_incomplete(hook.sent_at_ms)?;
            return Err(io::Error::other("Claude journal event too large"));
        }
        self.publish(&format!("{id}.event"), &bytes)
    }

    pub fn load(&self) -> io::Result<JournalReplay> {
        let _lock = self.lock()?;
        let mut replay = JournalReplay::default();
        let paths = match self.entries() {
            Ok(paths) => paths,
            Err(_) => {
                replay.incomplete = true;
                return Ok(replay);
            }
        };
        let mut acknowledged = Vec::new();
        let mut acknowledged_gaps = Vec::new();
        if let Some(path) = paths
            .iter()
            .rev()
            .find(|path| extension(path, "checkpoint"))
        {
            match read_json::<Checkpoint>(path, MAX_CHECKPOINT_BYTES) {
                Ok(checkpoint) => {
                    replay.checkpoint = Some(checkpoint.state);
                    acknowledged = checkpoint.acknowledged;
                    acknowledged_gaps = checkpoint.acknowledged_gaps;
                }
                Err(_) => replay.record_gap(path),
            }
        }
        for path in paths
            .iter()
            .filter(|path| extension(path, "gap") || extension(path, "pending"))
        {
            let token = path
                .file_name()
                .expect("journal entry has a file name")
                .to_string_lossy()
                .into_owned();
            if !acknowledged_gaps.contains(&token) {
                replay.record_gap(path);
            }
        }
        for path in paths.iter().filter(|path| extension(path, "event")) {
            let id = path
                .file_stem()
                .expect("event entry has a file stem")
                .to_string_lossy();
            let token = path
                .file_name()
                .expect("journal entry has a file name")
                .to_string_lossy()
                .into_owned();
            if acknowledged_gaps.contains(&token) {
                continue;
            }
            if acknowledged.iter().any(|ack| ack == id.as_ref()) {
                continue;
            }
            match read_json::<ClaudeHook>(path, MAX_EVENT_BYTES) {
                Ok(hook) if hook.delivery_id.as_deref() == Some(id.as_ref()) => {
                    replay.events.push(hook)
                }
                _ => replay.record_gap(path),
            }
        }
        replay.incomplete = !replay.gaps.is_empty();
        replay.events.sort_by(|left, right| {
            left.sent_at_ms
                .cmp(&right.sent_at_ms)
                .then_with(|| left.delivery_id.cmp(&right.delivery_id))
        });
        Ok(replay)
    }

    /// Publishes state and exact observed event/gap identities before cleanup.
    /// Pass `JournalReplay::gaps`; an empty list acknowledges no uncertainty.
    /// Concurrent arrivals and newer gap identities are never acknowledged.
    pub fn checkpoint(
        &self,
        state: &serde_json::Value,
        acknowledged: &[String],
        acknowledged_gaps: &[String],
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        if acknowledged.len() > MAX_EVENTS || acknowledged.iter().any(|id| !valid_id(id)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid journal acknowledgements",
            ));
        }
        if acknowledged_gaps.len() > MAX_ENTRIES
            || acknowledged_gaps.iter().any(|token| !valid_gap(token))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid gap acknowledgements",
            ));
        }
        let paths = self.entries()?;
        // Finish the preceding committed cleanup first. Otherwise a second
        // checkpoint could forget acknowledgements after a crash.
        if let Some(previous) = paths
            .iter()
            .rev()
            .find(|path| extension(path, "checkpoint"))
        {
            match read_json::<Checkpoint>(previous, MAX_CHECKPOINT_BYTES) {
                Ok(checkpoint) => {
                    self.remove_acknowledged(&checkpoint.acknowledged)?;
                    self.remove_gaps(&checkpoint.acknowledged_gaps)?;
                }
                Err(_)
                    if !acknowledged_gaps.contains(
                        &previous
                            .file_name()
                            .expect("journal entry has a file name")
                            .to_string_lossy()
                            .into_owned(),
                    ) =>
                {
                    self.mark_incomplete(gap_time_ms(previous))?
                }
                Err(_) => {}
            }
        }
        let generation = paths
            .iter()
            .filter(|path| extension(path, "checkpoint"))
            .filter_map(|path| path.file_stem()?.to_str()?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| io::Error::other("journal generation exhausted"))?;
        let bytes = serde_json::to_vec(&Checkpoint {
            state: state.clone(),
            acknowledged: acknowledged.to_vec(),
            acknowledged_gaps: acknowledged_gaps.to_vec(),
        })
        .map_err(io::Error::other)?;
        if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
            self.mark_incomplete(0)?;
            return Err(io::Error::other("Claude checkpoint too large"));
        }
        self.publish(&format!("{generation:020}.checkpoint"), &bytes)?;
        self.remove_acknowledged(acknowledged)?;
        self.remove_gaps(acknowledged_gaps)?;
        for path in paths {
            if extension(&path, "checkpoint") {
                remove_if_exists(&path)?;
            }
        }
        sync_directory(&self.directory)
    }

    fn remove_gaps(&self, gaps: &[String]) -> io::Result<()> {
        for token in gaps {
            if !valid_gap(token) {
                return Err(io::Error::other("invalid stored gap acknowledgement"));
            }
            remove_if_exists(&self.directory.join(token))?;
        }
        sync_directory(&self.directory)
    }

    fn remove_acknowledged(&self, acknowledged: &[String]) -> io::Result<()> {
        for id in acknowledged {
            if !valid_id(id) {
                return Err(io::Error::other("invalid stored acknowledgement"));
            }
            remove_if_exists(&self.directory.join(format!("{id}.event")))?;
        }
        sync_directory(&self.directory)
    }

    fn publish(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let destination = self.directory.join(name);
        if destination.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "journal identity collision",
            ));
        }
        let pending = self.directory.join(format!("{}.pending", unique_id()));
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // Destination is unique under the OS lock: no overwrite-rename,
        // including on Windows. An interrupted pending file means uncertainty.
        fs::rename(pending, &destination)?;
        #[cfg(windows)]
        OpenOptions::new()
            .write(true)
            .open(&destination)?
            .sync_all()?;
        sync_directory(&self.directory)
    }
}

impl JournalReplay {
    fn record_gap(&mut self, path: &Path) {
        let at_ms = gap_time_ms(path);
        self.gap_at_ms = if self.gaps.is_empty() {
            at_ms
        } else {
            merge_gap_time(self.gap_at_ms, at_ms)
        };
        self.gaps.push(
            path.file_name()
                .expect("journal entry has a file name")
                .to_string_lossy()
                .into_owned(),
        );
    }
}

fn merge_gap_time(left: u64, right: u64) -> u64 {
    if left == 0 || right == 0 {
        0
    } else {
        left.max(right)
    }
}

fn publication_time_ms(path: &Path) -> u64 {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.split_once('-'))
        .filter(|(time, _)| time.len() == 32)
        .and_then(|(time, _)| u128::from_str_radix(time, 16).ok())
        .and_then(|nanos| u64::try_from(nanos / 1_000_000).ok())
        .unwrap_or(0)
}

fn gap_time_ms(path: &Path) -> u64 {
    // Explicit dropped-event boundaries retain firing order, even if publication
    // was delayed. Zero is persisted too, so coalescing cannot erase uncertainty.
    if extension(path, "gap")
        && let Ok(at_ms) = read_json::<u64>(path, 32)
    {
        return at_ms;
    }
    // Legacy empty gaps and interrupted/corrupt records have no trusted event
    // stamp. Their immutable publication/modified time bounds the missing data,
    // unlike load time, which would keep advancing on every replay.
    let modified = fs::symlink_metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    publication_time_ms(path).max(modified)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

fn valid_gap(token: &str) -> bool {
    let Some((id, extension)) = token.rsplit_once('.') else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && matches!(extension, "gap" | "pending" | "event" | "checkpoint")
}

fn unique_id() -> String {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{time:032x}-{:08x}-{:016x}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn extension(path: &Path, expected: &str) -> bool {
    path.extension().is_some_and(|ext| ext == expected)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> io::Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io::Error::other("invalid journal file"));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("journal read limit exceeded"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn private_options() -> OpenOptions {
    let options = OpenOptions::new();
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut options = options;
        options.mode(0o600);
        options
    };
    options
}

fn private_directory(path: &Path) -> io::Result<()> {
    if !path.try_exists()? {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            && !parent.try_exists()?
        {
            private_directory(parent)?;
        }
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt as _;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        match builder.create(path) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            result => result?,
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            sync_directory(parent)?;
        }
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(io::Error::other("journal directory must not be a symlink"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    sync_directory(path)?;
    Ok(())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_gap_is_newer_than_pending_empty_inventory() {
        let (root, journal) = journal();
        let mut snapshot = ClaudeHook {
            sent_at_ms: 100,
            background_tasks: Some(Box::default()),
            session_crons: Some(Box::default()),
            ..Default::default()
        };
        journal.append(&mut snapshot).expect("valid test fixture");
        for index in 1..MAX_EVENTS {
            let id = format!("{index:x}");
            let hook = ClaudeHook {
                delivery_id: Some(id.clone()),
                sent_at_ms: 100,
                ..Default::default()
            };
            fs::write(
                journal.directory.join(format!("{id}.event")),
                serde_json::to_vec(&hook).expect("valid test fixture"),
            )
            .expect("valid test fixture");
        }
        let mut lost = ClaudeHook {
            event: "UserPromptSubmit".into(),
            sent_at_ms: 200,
            ..Default::default()
        };
        assert!(journal.append(&mut lost).is_err());
        let replay = journal.load().expect("valid test fixture");
        assert!(replay.incomplete);
        assert_eq!(replay.events.len(), MAX_EVENTS);
        assert_eq!(replay.gap_at_ms, 200);
        assert!(
            replay
                .events
                .iter()
                .all(|hook| hook.sent_at_ms < replay.gap_at_ms)
        );
        assert_eq!(journal.load().expect("valid test fixture").gap_at_ms, 200);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn old_gap_keeps_its_boundary_with_newer_replayed_inventory() {
        let (root, journal) = journal();
        journal.mark_incomplete(100).expect("valid test fixture");
        let observed = journal.load().expect("valid test fixture");
        let mut snapshot = ClaudeHook {
            sent_at_ms: 200,
            background_tasks: Some(Box::default()),
            session_crons: Some(Box::default()),
            ..Default::default()
        };
        journal.append(&mut snapshot).expect("valid test fixture");
        let replay = journal.load().expect("valid test fixture");
        assert_eq!(replay.gaps, observed.gaps);
        assert_eq!(replay.gap_at_ms, 100);
        assert!(replay.events[0].sent_at_ms > replay.gap_at_ms);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn arbitrary_pending_and_corrupt_records_have_stable_publication_boundaries() {
        let (root, journal) = journal();
        for name in ["interrupted.pending", "bad.event", "bad.checkpoint"] {
            fs::write(journal.directory.join(name), b"{").expect("valid test fixture");
        }
        let replay = journal.load().expect("valid test fixture");
        assert_eq!(replay.gaps.len(), 3);
        assert!(replay.gap_at_ms > 0);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let reopened =
            ClaudeJournal::in_directory(&root, "../pane/../../escape").expect("valid test fixture");
        let again = reopened.load().expect("valid test fixture");
        assert_eq!(again.gaps, replay.gaps);
        assert_eq!(again.gap_at_ms, replay.gap_at_ms);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn coalescing_out_of_order_losses_keeps_latest_boundary() {
        let (root, journal) = journal();
        journal.mark_incomplete(200).expect("valid test fixture");
        journal.mark_incomplete(100).expect("valid test fixture");
        assert_eq!(journal.load().expect("valid test fixture").gap_at_ms, 200);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn capacity_and_oversized_input_leave_explicit_uncertainty() {
        let (root, journal) = journal();
        let mut hook = ClaudeHook {
            cwd: Some("x".repeat(MAX_EVENT_BYTES as usize)),
            ..Default::default()
        };
        assert!(journal.append(&mut hook).is_err());
        assert!(journal.load().expect("valid test fixture").incomplete);
        for index in 0..MAX_EVENTS {
            fs::write(journal.directory.join(format!("{index:x}.event")), b"{}")
                .expect("valid test fixture");
        }
        assert!(journal.append(&mut ClaudeHook::default()).is_err());
        assert!(journal.load().expect("valid test fixture").incomplete);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn metadata_poll_is_initially_dirty_and_detects_arrivals() {
        let (root, mut journal) = journal();
        assert!(journal.changed().expect("valid test fixture"));
        assert!(!journal.changed().expect("valid test fixture"));
        journal
            .append(&mut ClaudeHook::default())
            .expect("valid test fixture");
        assert!(journal.changed().expect("valid test fixture"));
        assert!(!journal.changed().expect("valid test fixture"));
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    fn journal() -> (PathBuf, ClaudeJournal) {
        let root = std::env::temp_dir().join(format!("muxtrix-journal-{}", unique_id()));
        let journal =
            ClaudeJournal::in_directory(&root, "../pane/../../escape").expect("valid test fixture");
        assert!(journal.directory.starts_with(root.join("claude-activity")));
        (root, journal)
    }

    #[test]
    fn offline_events_survive_reopen_without_conversation_text() {
        let (root, journal) = journal();
        let mut hook = ClaudeHook {
            event: "Stop".into(),
            message: Some("secret".into()),
            last_assistant_message: Some("secret".into()),
            ..Default::default()
        };
        journal.append(&mut hook).expect("valid test fixture");
        let replay = ClaudeJournal::in_directory(&root, "../pane/../../escape")
            .expect("valid test fixture")
            .load()
            .expect("valid test fixture");
        assert_eq!(replay.events.len(), 1);
        assert_eq!(replay.events[0].delivery_id, hook.delivery_id);
        assert_eq!(replay.events[0].message, None);
        assert_eq!(replay.events[0].last_assistant_message, None);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn exact_acknowledgements_preserve_racing_arrivals() {
        let (root, journal) = journal();
        let mut first = ClaudeHook::default();
        journal.append(&mut first).expect("valid test fixture");
        let observed = journal.load().expect("valid test fixture");
        std::thread::scope(|scope| {
            for _ in 0..12 {
                scope.spawn(|| {
                    journal
                        .append(&mut ClaudeHook::default())
                        .expect("valid test fixture");
                });
            }
            journal
                .checkpoint(
                    &serde_json::json!({"running": true}),
                    &[observed.events[0]
                        .delivery_id
                        .clone()
                        .expect("valid test fixture")],
                    &observed.gaps,
                )
                .expect("valid test fixture");
        });
        let replay = journal.load().expect("valid test fixture");
        assert_eq!(replay.events.len(), 12);
        assert!(
            replay
                .events
                .iter()
                .all(|event| event.delivery_id != first.delivery_id)
        );
        assert_eq!(
            replay.checkpoint,
            Some(serde_json::json!({"running": true}))
        );
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn checkpoint_publish_before_cleanup_is_idempotent() {
        let (root, journal) = journal();
        let mut hook = ClaudeHook::default();
        journal.append(&mut hook).expect("valid test fixture");
        let checkpoint = Checkpoint {
            state: serde_json::json!({"state": "running"}),
            acknowledged: vec![hook.delivery_id.expect("valid test fixture")],
            acknowledged_gaps: vec![],
        };
        journal
            .publish(
                "00000000000000000001.checkpoint",
                &serde_json::to_vec(&checkpoint).expect("valid test fixture"),
            )
            .expect("valid test fixture");
        assert!(
            journal
                .load()
                .expect("valid test fixture")
                .events
                .is_empty()
        );
        journal
            .checkpoint(&checkpoint.state, &[], &[])
            .expect("valid test fixture");
        assert!(
            journal
                .load()
                .expect("valid test fixture")
                .events
                .is_empty()
        );
        assert_eq!(
            journal
                .entries()
                .expect("valid test fixture")
                .iter()
                .filter(|path| extension(path, "checkpoint"))
                .count(),
            1
        );
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn interrupted_and_corrupt_files_are_not_known_empty() {
        let (root, journal) = journal();
        fs::write(journal.directory.join("interrupted.pending"), b"{").expect("valid test fixture");
        assert!(journal.load().expect("valid test fixture").incomplete);
        journal
            .checkpoint(&serde_json::Value::Null, &[], &[])
            .expect("valid test fixture");
        assert!(journal.load().expect("valid test fixture").incomplete);
        fs::write(journal.directory.join("bad.event"), b"{").expect("valid test fixture");
        assert!(journal.load().expect("valid test fixture").incomplete);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn observed_gap_acknowledgement_allows_fresh_authoritative_evidence() {
        let (root, journal) = journal();
        fs::write(journal.directory.join("interrupted.pending"), b"{").expect("valid test fixture");
        let replay = journal.load().expect("valid test fixture");
        assert!(replay.incomplete);
        assert_eq!(replay.gaps, ["interrupted.pending"]);
        journal
            .checkpoint(&serde_json::json!({"state":"unknown"}), &[], &replay.gaps)
            .expect("valid test fixture");
        let mut hook = ClaudeHook::from_payload(
            &serde_json::json!({
                "session_id":"fresh-session", "background_tasks":[], "session_crons":[]
            }),
            "Stop",
        );
        journal.append(&mut hook).expect("valid test fixture");
        let replay = journal.load().expect("valid test fixture");
        assert!(!replay.incomplete);
        assert!(replay.gaps.is_empty());
        assert_eq!(
            replay.events[0].session_id.as_deref(),
            Some("fresh-session")
        );
        assert_eq!(replay.events[0].background_tasks, Some(Box::default()));
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn acknowledging_observed_gap_never_acknowledges_concurrent_gap() {
        let (root, journal) = journal();
        journal.mark_incomplete(100).expect("valid test fixture");
        let observed = journal.load().expect("valid test fixture");
        journal.mark_incomplete(200).expect("valid test fixture");
        journal
            .checkpoint(&serde_json::Value::Null, &[], &observed.gaps)
            .expect("valid test fixture");
        let newer = journal.load().expect("valid test fixture");
        assert!(newer.incomplete);
        assert_ne!(newer.gaps, observed.gaps);
        assert_eq!(newer.gap_at_ms, 200);
        journal
            .checkpoint(&serde_json::Value::Null, &[], &newer.gaps)
            .expect("valid test fixture");
        assert!(!journal.load().expect("valid test fixture").incomplete);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[test]
    fn gap_checkpoint_is_effective_before_cleanup() {
        let (root, journal) = journal();
        fs::write(journal.directory.join("interrupted.pending"), b"{").expect("valid test fixture");
        let observed = journal.load().expect("valid test fixture");
        let checkpoint = Checkpoint {
            state: serde_json::json!({"state":"unknown"}),
            acknowledged: vec![],
            acknowledged_gaps: observed.gaps,
        };
        journal
            .publish(
                "00000000000000000001.checkpoint",
                &serde_json::to_vec(&checkpoint).expect("valid test fixture"),
            )
            .expect("valid test fixture");
        assert!(!journal.load().expect("valid test fixture").incomplete);
        journal
            .checkpoint(&checkpoint.state, &[], &[])
            .expect("valid test fixture");
        assert!(!journal.load().expect("valid test fixture").incomplete);
        fs::remove_dir_all(root).expect("valid test fixture");
    }

    #[cfg(unix)]
    #[test]
    fn directories_and_events_are_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let (root, journal) = journal();
        journal
            .append(&mut ClaudeHook::default())
            .expect("valid test fixture");
        assert_eq!(
            fs::metadata(&journal.directory)
                .expect("valid test fixture")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for path in journal.entries().expect("valid test fixture") {
            assert_eq!(
                fs::metadata(path)
                    .expect("valid test fixture")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).expect("valid test fixture");
    }
}
