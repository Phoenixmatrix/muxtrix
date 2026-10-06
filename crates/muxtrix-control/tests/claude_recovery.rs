use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use muxtrix_control::ClaudeHook;
use muxtrix_control::claude_journal::ClaudeJournal;
use serde_json::{Value, json};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OfflineRegistry {
    root: PathBuf,
}

impl OfflineRegistry {
    fn new() -> Result<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "muxtrix-claude-recovery-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        // The journal creates its private directory. No listener or route is
        // registered here, even when the test runner is inside a live pane.
        Ok(Self { root })
    }

    fn journal(&self, pane: &str) -> Result<ClaudeJournal> {
        Ok(ClaudeJournal::in_directory(&self.root, pane)?)
    }

    fn spawn_hook(&self, pane: &str, stamp: u64, payload: &Value) -> Result<Child> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_muxtrixctl"));
        command
            .args(["hook-event", "--agent", "claude", "--state", "idle"])
            .arg("--fired-at-ms")
            .arg(stamp.to_string())
            .env("MUXTRIX_PANE_ID", pane)
            .env("MUXTRIX_CONTROL_REGISTRY", &self.root)
            .env_remove("MUXTRIX_CONTROL_ENDPOINT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().expect("piped hook stdin");
        let written = stdin.write_all(&serde_json::to_vec(payload)?);
        drop(stdin);
        if let Err(error) = written {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        Ok(child)
    }

    fn hook(&self, pane: &str, stamp: u64, payload: &Value) -> Result {
        finish_hook(self.spawn_hook(pane, stamp, payload)?)
    }
}

impl Drop for OfflineRegistry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn finish_hook(child: Child) -> Result {
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Claude's hook protocol accepts a JSON object; no diagnostic wording is
    // part of this regression's contract.
    assert!(serde_json::from_slice::<Value>(&output.stdout)?.is_object());
    Ok(())
}

fn stop(session: &str, status: &str) -> Value {
    json!({
        "hook_event_name": "Stop",
        "session_id": session,
        "cwd": "/project",
        "transcript_path": "/project/session.jsonl",
        "stop_hook_active": false,
        "last_assistant_message": "private conversation text",
        "background_tasks": [{
            "id": "shell-1",
            "type": "shell",
            "status": status,
            "ambient": false
        }],
        "session_crons": []
    })
}

fn delivery(hook: &ClaudeHook) -> String {
    let id = hook
        .delivery_id
        .as_ref()
        .expect("durable delivery identity");
    assert!(!id.is_empty());
    id.clone()
}

#[test]
fn offline_stop_and_completion_survive_checkpoint_arrivals() -> Result {
    let registry = OfflineRegistry::new()?;
    let pane = "offline-pane";
    registry.hook(pane, 10, &stop("session-a", "running"))?;

    // Reopen only after the real CLI has exited: recovery must not depend on
    // that process, its stdin, or a GUI ever having received the event.
    let journal = registry.journal(pane)?;
    let observed = journal.load()?;
    assert!(!observed.incomplete);
    assert!(observed.checkpoint.is_none());
    assert_eq!(observed.events.len(), 1);
    let running = &observed.events[0];
    assert_eq!(running.event, "Stop");
    assert_eq!(running.session_id.as_deref(), Some("session-a"));
    assert_eq!(running.cwd.as_deref(), Some("/project"));
    assert_eq!(
        running.transcript_path.as_deref(),
        Some("/project/session.jsonl")
    );
    assert_eq!(running.sent_at_ms, 10);
    assert!(!running.background_tasks_invalid);
    let tasks = running.background_tasks.as_ref().expect("task snapshot");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "shell-1");
    assert_eq!(tasks[0].kind, "shell");
    assert_eq!(tasks[0].status, "running");
    assert_eq!(tasks[0].ambient, Some(false));
    assert_eq!(running.session_crons, Some(Box::default()));
    assert!(running.last_assistant_message.is_none());

    // A completion arriving after the consumer's read but before its commit
    // must not be swept up by an acknowledgement of the running snapshot.
    registry.hook(pane, 20, &stop("session-a", "completed"))?;
    let running_state = serde_json::to_value(running)?;
    journal.checkpoint(&running_state, &[delivery(running)], &observed.gaps)?;
    let recovered = registry.journal(pane)?.load()?;
    assert!(!recovered.incomplete);
    assert_eq!(recovered.checkpoint, Some(running_state));
    assert_eq!(recovered.events.len(), 1);
    let completed = &recovered.events[0];
    assert_eq!(completed.sent_at_ms, 20);
    assert_ne!(delivery(completed), delivery(running));
    assert_eq!(
        completed
            .background_tasks
            .as_ref()
            .expect("valid test fixture")[0]
            .status,
        "completed"
    );

    // Another process arrives after that checkpoint. A further completion
    // races the next checkpoint; either lock ordering must retain both new
    // deliveries, since neither was in that checkpoint's observed read set.
    registry.hook(pane, 30, &stop("session-a", "running"))?;
    let concurrent = registry.spawn_hook(pane, 40, &stop("session-a", "completed"))?;
    let completed_state = serde_json::to_value(completed)?;
    let checkpoint = journal.checkpoint(&completed_state, &[delivery(completed)], &recovered.gaps);
    finish_hook(concurrent)?;
    checkpoint?;
    let recovered = registry.journal(pane)?.load()?;
    assert!(!recovered.incomplete);
    assert_eq!(recovered.checkpoint, Some(completed_state));
    assert_eq!(recovered.events.len(), 2);
    assert_eq!(recovered.events[0].sent_at_ms, 30);
    assert_eq!(recovered.events[1].sent_at_ms, 40);
    assert_eq!(
        recovered.events[0]
            .background_tasks
            .as_ref()
            .expect("valid test fixture")[0]
            .status,
        "running"
    );
    assert_eq!(
        recovered.events[1]
            .background_tasks
            .as_ref()
            .expect("valid test fixture")[0]
            .status,
        "completed"
    );
    let ids: Vec<_> = recovered.events.iter().map(delivery).collect();
    assert_ne!(ids[0], ids[1]);
    let final_state = serde_json::to_value(&recovered.events[1])?;
    journal.checkpoint(&final_state, &ids, &recovered.gaps)?;
    let final_replay = registry.journal(pane)?.load()?;
    assert!(!final_replay.incomplete);
    assert!(final_replay.events.is_empty());
    assert_eq!(final_replay.checkpoint, Some(final_state));
    Ok(())
}

#[test]
fn offline_acknowledgements_are_isolated_by_pane_and_delivery_not_session() -> Result {
    let registry = OfflineRegistry::new()?;
    // Deliberately reuse session/task identities across panes and task identity
    // across sessions, so isolation cannot accidentally rely on those keys.
    registry.hook("pane-a", 10, &stop("shared-session", "running"))?;
    registry.hook("pane-b", 10, &stop("shared-session", "running"))?;
    let pane_a = registry.journal("pane-a")?;
    let pane_b = registry.journal("pane-b")?;
    let initial_a = pane_a.load()?;
    let initial_b = pane_b.load()?;
    assert_eq!(initial_a.events.len(), 1);
    assert_eq!(initial_b.events.len(), 1);
    assert_ne!(
        delivery(&initial_a.events[0]),
        delivery(&initial_b.events[0])
    );
    registry.hook("pane-a", 20, &stop("new-session", "completed"))?;
    pane_a.checkpoint(
        &serde_json::to_value(&initial_a.events[0])?,
        &[delivery(&initial_a.events[0])],
        &initial_a.gaps,
    )?;
    let recovered_a = registry.journal("pane-a")?.load()?;
    let recovered_b = registry.journal("pane-b")?.load()?;
    assert!(!recovered_a.incomplete);
    assert!(!recovered_b.incomplete);
    assert_eq!(recovered_a.events.len(), 1);
    assert_eq!(
        recovered_a.events[0].session_id.as_deref(),
        Some("new-session")
    );
    assert_eq!(
        recovered_a.events[0]
            .background_tasks
            .as_ref()
            .expect("valid test fixture")[0]
            .status,
        "completed"
    );
    assert_eq!(recovered_b.events, initial_b.events);
    assert!(recovered_b.checkpoint.is_none());
    pane_b.checkpoint(
        &serde_json::to_value(&recovered_b.events[0])?,
        &[delivery(&recovered_b.events[0])],
        &recovered_b.gaps,
    )?;
    assert_eq!(
        registry.journal("pane-a")?.load()?.events,
        recovered_a.events
    );
    assert!(registry.journal("pane-b")?.load()?.events.is_empty());
    Ok(())
}

#[test]
fn offline_malformed_snapshots_remain_unknown_in_replay_and_checkpoint() -> Result {
    let registry = OfflineRegistry::new()?;
    let payloads = [
        json!({"hook_event_name":"Stop", "session_id":"session-a"}),
        json!({"hook_event_name":"Stop", "session_id":"session-a",
            "background_tasks":{"id":"shell-1"}, "session_crons":null}),
        json!({"hook_event_name":"Stop", "session_id":"session-a",
            "background_tasks":[{"id":"shell-1"}], "session_crons":[{"id":"cron-1"}]}),
        json!({"hook_event_name":"Stop", "session_id":"session-a",
            "background_tasks":[], "session_crons":[]}),
    ];
    for (index, payload) in payloads.iter().enumerate() {
        registry.hook("pane", index as u64 + 1, payload)?;
    }
    let journal = registry.journal("pane")?;
    let replay = journal.load()?;
    assert!(!replay.incomplete);
    assert_eq!(replay.events.len(), 4);
    let absent = &replay.events[0];
    assert_eq!(absent.background_tasks, None);
    assert!(!absent.background_tasks_invalid);
    assert_eq!(absent.session_crons, None);
    assert!(!absent.session_crons_invalid);
    let malformed = &replay.events[1];
    assert_eq!(malformed.background_tasks, None);
    assert!(malformed.background_tasks_invalid);
    assert_eq!(malformed.session_crons, None);
    assert!(malformed.session_crons_invalid);
    let partial = &replay.events[2];
    assert_eq!(partial.background_tasks, Some(Box::default()));
    assert!(partial.background_tasks_invalid);
    assert_eq!(partial.session_crons, Some(Box::default()));
    assert!(partial.session_crons_invalid);
    let empty = &replay.events[3];
    assert_eq!(empty.background_tasks, Some(Box::default()));
    assert!(!empty.background_tasks_invalid);
    assert_eq!(empty.session_crons, Some(Box::default()));
    assert!(!empty.session_crons_invalid);

    // The journal's checkpoint state is consumer-owned JSON. Round-trip the
    // typed evidence through it rather than replacing uncertainty with idle.
    let ids: Vec<_> = replay.events.iter().map(delivery).collect();
    journal.checkpoint(&serde_json::to_value(&replay.events)?, &ids, &replay.gaps)?;
    let recovered = registry.journal("pane")?.load()?;
    assert!(!recovered.incomplete);
    assert!(recovered.events.is_empty());
    let restored: Vec<ClaudeHook> =
        serde_json::from_value(recovered.checkpoint.expect("valid test fixture"))?;
    assert_eq!(restored, replay.events);
    Ok(())
}
