//! Session persistence: a per-session daemon owns every PTY so shells and
//! agents keep running when the GUI closes, tmux-style. The GUI attaches
//! over a local socket, replays each pane's ring-buffered backlog into a
//! fresh VT, and streams from there. One daemon process per session; a
//! JSON registry under `~/.muxtrix/sessions` lists what exists.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, SendHalf, Stream, ToFsName as _, ToNsName as _,
    traits::Stream as _,
};
use muxtrix_platform::PtyOutput;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Cap on the per-pane backlog replayed at attach. Enough for meaningful
/// scrollback reconstruction without unbounded daemon growth.
const BACKLOG_LIMIT: usize = 512 * 1024;

pub mod daemon;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    Attach,
    Spawn {
        pane: Uuid,
        executable: String,
        arguments: Vec<String>,
        working_directory: Option<PathBuf>,
        environment: Vec<(String, String)>,
        rows: u16,
        cols: u16,
    },
    Input {
        pane: Uuid,
        data: String,
    },
    Resize {
        pane: Uuid,
        rows: u16,
        cols: u16,
    },
    Kill {
        pane: Uuid,
    },
    /// Unlike Kill, this requires a correlated confirmation of child exit.
    KillAndWait {
        pane: Uuid,
        request: Uuid,
        generation: u64,
    },
    Layout {
        data: String,
    },
    Rename {
        name: String,
    },
    /// Client is leaving; the daemon drops its connection halves so the
    /// client's blocked reader unblocks (split halves share one fd, so a
    /// dropped SendHalf alone never reads as EOF).
    Detach,
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Attached {
        panes: Vec<PaneSummary>,
        layout: Option<String>,
    },
    Backlog {
        pane: Uuid,
        data: String,
    },
    /// Legacy replay boundary retained for older clients. Output provenance is
    /// carried by Backlog and Output themselves, not by consuming this marker.
    BacklogDone {
        pane: Uuid,
    },
    Output {
        pane: Uuid,
        data: String,
    },
    Exited {
        pane: Uuid,
        clean: bool,
    },
    Spawned {
        pane: Uuid,
        process_id: Option<u32>,
        #[serde(default)]
        generation: Option<u64>,
    },
    SpawnFailed {
        pane: Uuid,
        error: String,
    },
    KillCompleted {
        request: Uuid,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneSummary {
    pub pane: Uuid,
    pub exited: Option<bool>,
    #[serde(default)]
    pub generation: Option<u64>,
}

/// One line in the on-disk registry: everything a client needs to list,
/// attach to, or kill a session without talking to it first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: Uuid,
    pub name: String,
    pub endpoint: String,
    pub process_id: u32,
    pub created_unix: u64,
    #[serde(default)]
    pub layout: Option<String>,
    /// Whether a GUI is currently attached. Set when a client sends Attach
    /// (mere socket probes do not count) and cleared when it leaves.
    #[serde(default)]
    pub attached: bool,
    /// The Muxtrix version the daemon runs — a long-lived daemon can
    /// outlive several app updates, and clients surface the skew.
    #[serde(default)]
    pub version: String,
}

pub fn sessions_directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".muxtrix").join("sessions"))
}

pub fn registry_path(id: Uuid) -> Option<PathBuf> {
    Some(sessions_directory()?.join(format!("{id}.json")))
}

/// Every session on record, dead ones included — callers decide liveness
/// via `record_is_alive`.
pub fn list_sessions() -> Vec<SessionRecord> {
    let Some(dir) = sessions_directory() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut records: Vec<SessionRecord> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|content| serde_json::from_str(&content).ok())
        .collect();
    records.sort_by_key(|record: &SessionRecord| record.created_unix);
    records
}

pub fn remove_session_record(id: Uuid) {
    if let Some(path) = registry_path(id) {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether the daemon process behind a record still exists. A dead record
/// is stale registry residue, not a resumable session.
pub fn record_is_alive(record: &SessionRecord) -> bool {
    connect(&record.endpoint).is_ok()
}

/// Sessions worth offering at startup: alive, and with no GUI attached.
pub fn resumable_sessions(own: Option<Uuid>) -> Vec<SessionRecord> {
    list_sessions()
        .into_iter()
        .filter(|record| Some(record.id) != own)
        .filter(|record| !record.attached)
        .filter(record_is_alive)
        .collect()
}

fn socket_name(endpoint: &str) -> std::io::Result<interprocess::local_socket::Name<'_>> {
    if cfg!(windows) {
        endpoint.to_ns_name::<GenericNamespaced>()
    } else {
        endpoint.to_fs_name::<GenericFilePath>()
    }
}

fn connect(endpoint: &str) -> std::io::Result<Stream> {
    Stream::connect(socket_name(endpoint)?)
}

/// The default endpoint address for a session id.
pub fn session_endpoint(id: Uuid) -> String {
    if cfg!(windows) {
        format!("muxtrix-session-{id}")
    } else {
        sessions_directory()
            .map_or_else(
                || std::env::temp_dir().join(format!("muxtrix-session-{id}.sock")),
                |dir| dir.join(format!("{id}.sock")),
            )
            .to_string_lossy()
            .into_owned()
    }
}

/// Client half: owns the socket, demultiplexes daemon events per pane, and
/// hands each pane output packets with replay provenance plus control handles.
pub struct SessionClient {
    writer: Arc<Mutex<SendHalf>>,
    events: Mutex<Receiver<Event>>,
    pane_outputs: Arc<Mutex<HashMap<Uuid, Sender<PtyOutput>>>>,
    /// Updated under `pane_outputs` so registration cannot outlive disconnect.
    connected: Arc<AtomicBool>,
    pane_exits: Arc<Mutex<HashMap<Uuid, bool>>>,
    pane_pids: Arc<Mutex<HashMap<Uuid, u32>>>,
    /// Why the host could not start a pane's process. Without this a failed
    /// spawn is indistinguishable from a pane that simply never printed:
    /// the request was written successfully, and the failure arrives later
    /// as an event nobody reads.
    pane_spawn_failures: Arc<Mutex<HashMap<Uuid, String>>>,
    kill_waiters: KillWaiters,
    pane_generations: Arc<Mutex<HashMap<Uuid, u64>>>,
}

type KillWaiters = Arc<Mutex<HashMap<Uuid, Sender<Result<(), String>>>>>;

/// Publishes a pane's terminal state before closing its output stream.
///
/// The terminal actor treats a closed output channel as PTY EOF and immediately
/// asks the client whether that EOF was a clean exit. Dropping the sender first
/// therefore races the actor: it can observe EOF before `pane_exits` is filled
/// and retain an ordinary `exit` as an unresponsive, unclean pane.
fn finish_tracked_pane(
    outputs: &Mutex<HashMap<Uuid, Sender<PtyOutput>>>,
    exits: &Mutex<HashMap<Uuid, bool>>,
    spawn_failures: &Mutex<HashMap<Uuid, String>>,
    pane: Uuid,
    clean: bool,
    spawn_failure: Option<&str>,
) {
    // Hold the output registry until every piece of state queried after EOF is
    // visible. `register_pane` and `unregister_pane` use this same lock as the
    // lifecycle boundary, so an already-forgotten pane is never regrown.
    let mut outputs = outputs.lock().expect("outputs");
    if !outputs.contains_key(&pane) {
        return;
    }
    exits.lock().expect("exits").insert(pane, clean);
    if let Some(error) = spawn_failure {
        spawn_failures
            .lock()
            .expect("spawn failures")
            .insert(pane, error.to_owned());
    }
    outputs.remove(&pane);
}

impl SessionClient {
    pub fn connect_endpoint(
        endpoint: &str,
    ) -> std::io::Result<(Self, Vec<PaneSummary>, Option<String>)> {
        let stream = connect(endpoint)?;
        let (read_half, mut writer) = Stream::split(stream);
        let (event_tx, event_rx) = mpsc::channel();
        let pane_outputs: Arc<Mutex<HashMap<Uuid, Sender<PtyOutput>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let connected = Arc::new(AtomicBool::new(true));
        let reader_connected = Arc::clone(&connected);
        let pane_exits: Arc<Mutex<HashMap<Uuid, bool>>> = Arc::new(Mutex::new(HashMap::new()));
        let pane_pids: Arc<Mutex<HashMap<Uuid, u32>>> = Arc::new(Mutex::new(HashMap::new()));
        let pane_spawn_failures: Arc<Mutex<HashMap<Uuid, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let kill_waiters: KillWaiters = Arc::new(Mutex::new(HashMap::new()));
        let waiters = Arc::clone(&kill_waiters);
        let pane_generations = Arc::new(Mutex::new(HashMap::new()));
        let generations = Arc::clone(&pane_generations);
        let outputs = Arc::clone(&pane_outputs);
        let exits = Arc::clone(&pane_exits);
        let pids = Arc::clone(&pane_pids);
        let spawn_failures = Arc::clone(&pane_spawn_failures);
        let reader = BufReader::new(read_half);
        std::thread::Builder::new()
            .name("muxtrix-session-client".into())
            .spawn(move || {
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let Ok(event) = serde_json::from_str::<Event>(&line) else {
                        continue;
                    };
                    match &event {
                        Event::Attached { panes, .. } => {
                            let mut generations = generations.lock().expect("generations");
                            for pane in panes {
                                if let Some(generation) = pane.generation {
                                    generations.insert(pane.pane, generation);
                                }
                            }
                        }
                        Event::KillCompleted { request, error } => {
                            if let Some(waiter) =
                                waiters.lock().expect("kill waiters").remove(request)
                            {
                                let _ = waiter.send(error.clone().map_or(Ok(()), Err));
                            }
                            continue;
                        }
                        Event::Backlog { pane, data } => {
                            if let Ok(bytes) = BASE64.decode(data)
                                && let Some(sender) = outputs.lock().expect("outputs").get(pane)
                            {
                                let _ = sender.send(PtyOutput::Backlog(bytes));
                            }
                            continue;
                        }
                        Event::BacklogDone { .. } => {
                            continue;
                        }
                        Event::Output { pane, data } => {
                            if let Ok(bytes) = BASE64.decode(data)
                                && let Some(sender) = outputs.lock().expect("outputs").get(pane)
                            {
                                let _ = sender.send(PtyOutput::Live(bytes));
                            }
                            continue;
                        }
                        Event::Exited { pane, clean } => {
                            finish_tracked_pane(
                                &outputs,
                                &exits,
                                &spawn_failures,
                                *pane,
                                *clean,
                                None,
                            );
                            continue;
                        }
                        Event::Spawned {
                            pane,
                            process_id,
                            generation,
                        } => {
                            if let Some(generation) = generation {
                                generations
                                    .lock()
                                    .expect("generations")
                                    .insert(*pane, *generation);
                            }
                            if let Some(process_id) = process_id {
                                pids.lock().expect("pids").insert(*pane, *process_id);
                            }
                            continue;
                        }
                        Event::SpawnFailed { pane, error } => {
                            // Failure reads as an unclean immediate exit;
                            // dropping the sender is the reader's EOF. Record
                            // the reason first so the pane can say why it is
                            // empty instead of looking like a live terminal.
                            finish_tracked_pane(
                                &outputs,
                                &exits,
                                &spawn_failures,
                                *pane,
                                false,
                                Some(error),
                            );
                            continue;
                        }
                    }
                    if event_tx.send(event).is_err() {
                        break;
                    }
                }
                // Losing the connection is not confirmation of process exit.
                // Wake existing readers without inventing an exit verdict.
                // Serialize with registration so late readers close too.
                let mut outputs = outputs.lock().expect("outputs");
                reader_connected.store(false, Ordering::Release);
                outputs.clear();
                drop(outputs);
                waiters.lock().expect("kill waiters").clear();
            })
            .map_err(std::io::Error::other)?;
        send_line(&mut writer, &Request::Attach)?;
        let (panes, layout) = match event_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Event::Attached { panes, layout }) => (panes, layout),
            _ => (Vec::new(), None),
        };
        Ok((
            Self {
                writer: Arc::new(Mutex::new(writer)),
                events: Mutex::new(event_rx),
                pane_outputs,
                connected,
                pane_exits,
                pane_pids,
                pane_spawn_failures,
                kill_waiters,
                pane_generations,
            },
            panes,
            layout,
        ))
    }

    pub fn send(&self, request: &Request) -> std::io::Result<()> {
        send_line(&mut self.writer.lock().expect("session writer"), request)
    }

    /// Blocks for a correlated daemon acknowledgment, never for a generic
    /// pane exit (which may belong to a previous incarnation). Worker-only.
    pub fn kill_and_wait(&self, pane: Uuid) -> Result<(), String> {
        self.kill_and_wait_timeout(pane, Duration::from_secs(7))
    }

    fn kill_and_wait_timeout(&self, pane: Uuid, timeout: Duration) -> Result<(), String> {
        let generation = self.pane_generations.lock().expect("generations").get(&pane).copied()
            .ok_or_else(|| "daemon cannot identify this pane incarnation; restart the session host to enable safe task cleanup (or wait for pane startup)".to_owned())?;
        let request = Uuid::new_v4();
        let (sender, receiver) = mpsc::channel();
        self.kill_waiters
            .lock()
            .expect("kill waiters")
            .insert(request, sender.clone());
        let writer = Arc::clone(&self.writer);
        // A blocked socket write or contended writer must not defeat the
        // acknowledgment deadline. Ordinary sends keep their existing path.
        let sent = std::thread::Builder::new()
            .name("muxtrix-kill-request".into())
            .spawn(move || {
                if let Err(error) = send_line(
                    &mut writer.lock().expect("session writer"),
                    &Request::KillAndWait {
                        pane,
                        request,
                        generation,
                    },
                ) {
                    let _ = sender.send(Err(error.to_string()));
                }
            });
        let result = match sent {
            Ok(_) => receiver.recv_timeout(timeout).map_err(|error| {
                format!("daemon did not confirm process termination (unsupported or disconnected host): {error}")
            }).and_then(|result| result),
            Err(error) => Err(error.to_string()),
        };
        self.kill_waiters
            .lock()
            .expect("kill waiters")
            .remove(&request);
        result
    }

    pub fn try_event(&self) -> Result<Event, TryRecvError> {
        self.events.lock().expect("session events").try_recv()
    }

    /// Clears state attached to one pane incarnation. Callers hold
    /// `pane_outputs` across this operation so exit delivery cannot race the
    /// transition to a replacement or unregistered pane.
    fn clear_pane_metadata(&self, pane: Uuid) {
        self.pane_exits.lock().expect("exits").remove(&pane);
        self.pane_pids.lock().expect("pids").remove(&pane);
        self.pane_spawn_failures
            .lock()
            .expect("spawn failures")
            .remove(&pane);
    }

    /// Registers a pane and returns its output packets. Each packet retains
    /// whether it is historical or live, even when consumption is delayed.
    ///
    /// Pane ids are durable, so this is also how a replacement claims the id
    /// of the pane it replaces. Everything recorded about the previous
    /// incarnation is dropped here: an inherited exit makes the replacement
    /// report itself dead the moment it is polled.
    pub fn register_pane(&self, pane: Uuid) -> Receiver<PtyOutput> {
        let (sender, receiver) = mpsc::channel();
        // This lock is the pane lifecycle boundary. Holding it while stale
        // metadata is cleared prevents a concurrent exit event from leaving
        // the replacement with its predecessor's status.
        let mut outputs = self.pane_outputs.lock().expect("outputs");
        self.clear_pane_metadata(pane);
        if self.connected.load(Ordering::Acquire) {
            outputs.insert(pane, sender);
        }
        receiver
    }

    /// Forgets a pane the GUI has closed. Dropping the pane's output sender
    /// is the EOF its reader thread blocks on — a pane dropped without this
    /// strands that thread, and its bookkeeping, for the life of the process.
    pub fn unregister_pane(&self, pane: Uuid) {
        // Keep EOF last here as well. If the reader wakes while this method is
        // still clearing state, it must never find stale exit metadata.
        let mut outputs = self.pane_outputs.lock().expect("outputs");
        self.clear_pane_metadata(pane);
        self.pane_generations
            .lock()
            .expect("generations")
            .remove(&pane);
        outputs.remove(&pane);
    }

    /// Whether the client is still streaming this pane. False once the pane
    /// has exited or been unregistered — in both cases its output channel is
    /// closed and its reader has ended.
    pub fn tracks_pane(&self, pane: Uuid) -> bool {
        self.pane_outputs
            .lock()
            .expect("outputs")
            .contains_key(&pane)
    }

    pub fn pane_exit(&self, pane: Uuid) -> Option<bool> {
        self.pane_exits.lock().expect("exits").get(&pane).copied()
    }

    pub fn pane_process_id(&self, pane: Uuid) -> Option<u32> {
        self.pane_pids.lock().expect("pids").get(&pane).copied()
    }

    /// Why the host refused to start this pane's process, when it did.
    pub fn pane_spawn_failure(&self, pane: Uuid) -> Option<String> {
        self.pane_spawn_failures
            .lock()
            .expect("spawn failures")
            .get(&pane)
            .cloned()
    }
}

impl Drop for SessionClient {
    fn drop(&mut self) {
        let _ = self.send(&Request::Detach);
    }
}

fn send_line(writer: &mut SendHalf, request: &Request) -> std::io::Result<()> {
    let mut line = serde_json::to_string(request).map_err(std::io::Error::other)?;
    line.push('\n');
    writer.write_all(line.as_bytes())
}

pub fn encode_bytes(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn with_shutdown_peer(
        generation: Option<u64>,
        reply: &str,
        check: impl FnOnce(&SessionClient, Uuid),
    ) {
        use interprocess::local_socket::{ListenerOptions, traits::Listener as _};
        let pane = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("muxtrix-kill-peer-{pane}"));
        std::fs::create_dir_all(&dir).expect("directory");
        let endpoint = dir.join("peer.sock").to_string_lossy().into_owned();
        let listener = ListenerOptions::new()
            .name(socket_name(&endpoint).expect("name"))
            .create_sync()
            .expect("listener");
        let reply = reply.to_owned();
        let peer = std::thread::spawn(move || {
            let (reader, mut writer) = Stream::split(listener.accept().expect("connection"));
            for line in BufReader::new(reader).lines() {
                let request: Request =
                    serde_json::from_str(&line.expect("request line")).expect("request");
                let event = match request {
                    Request::Attach => Event::Attached {
                        panes: vec![PaneSummary {
                            pane,
                            exited: None,
                            generation,
                        }],
                        layout: None,
                    },
                    Request::KillAndWait { request, .. } => {
                        assert!(
                            generation.is_some(),
                            "legacy host must never receive an unsupported request"
                        );
                        match reply.as_str() {
                            "disconnect" => break,
                            "error" => Event::KillCompleted {
                                request,
                                error: Some("permission denied".into()),
                            },
                            _ => {
                                // Neither a stale pane exit nor someone else's
                                // successful request may unlock cleanup.
                                writeln!(
                                    writer,
                                    "{}",
                                    serde_json::to_string(&Event::Exited { pane, clean: true })
                                        .expect("exit")
                                )
                                .expect("exit write");
                                Event::KillCompleted {
                                    request: Uuid::new_v4(),
                                    error: None,
                                }
                            }
                        }
                    }
                    Request::Detach => break,
                    _ => continue,
                };
                writeln!(writer, "{}", serde_json::to_string(&event).expect("event"))
                    .expect("reply");
            }
        });
        let (client, _, _) = SessionClient::connect_endpoint(&endpoint).expect("client");
        check(&client, pane);
        drop(client);
        peer.join().expect("peer");
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    #[cfg(unix)]
    fn acknowledged_kill_rejects_legacy_daemon_without_incarnation_support() {
        with_shutdown_peer(None, "ignore", |client, pane| {
            assert!(client.kill_and_wait(pane).is_err());
            client
                .send(&Request::Attach)
                .expect("legacy connection stays open");
            assert!(matches!(
                client
                    .events
                    .lock()
                    .expect("events")
                    .recv_timeout(Duration::from_secs(1)),
                Ok(Event::Attached { .. })
            ));
        });
    }

    #[test]
    #[cfg(unix)]
    fn acknowledged_kill_times_out_on_unrelated_exit_and_acknowledgment() {
        with_shutdown_peer(Some(1), "ignore", |client, pane| {
            let start = std::time::Instant::now();
            assert!(
                client
                    .kill_and_wait_timeout(pane, Duration::from_millis(100))
                    .is_err()
            );
            assert!(start.elapsed() < Duration::from_secs(2));
        });
    }

    #[test]
    #[cfg(unix)]
    fn acknowledged_kill_propagates_daemon_error() {
        with_shutdown_peer(Some(1), "error", |client, pane| {
            assert_eq!(client.kill_and_wait(pane), Err("permission denied".into()));
        });
    }

    #[test]
    #[cfg(unix)]
    fn acknowledged_kill_rejects_connection_closure() {
        with_shutdown_peer(Some(1), "disconnect", |client, pane| {
            assert!(client.kill_and_wait(pane).is_err());
        });
    }

    #[test]
    #[cfg(unix)]
    fn connection_loss_closes_existing_and_late_pane_readers_without_claiming_exit() {
        with_shutdown_peer(Some(1), "disconnect", |client, pane| {
            let output = client.register_pane(pane);
            assert!(client.kill_and_wait(pane).is_err());
            assert!(matches!(
                output.recv_timeout(Duration::from_secs(1)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ));
            // A lost transport does not prove the child died, and must never
            // authorize worktree deletion or mark a pane as a clean exit.
            assert_eq!(client.pane_exit(pane), None);
            let late = client.register_pane(Uuid::new_v4());
            assert!(matches!(
                late.recv_timeout(Duration::from_secs(1)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ));
        });
    }

    #[test]
    fn requests_and_events_round_trip_as_json_lines() {
        let request = Request::Input {
            pane: Uuid::nil(),
            data: encode_bytes(b"ls\r"),
        };
        let json = serde_json::to_string(&request).expect("serialize");
        assert!(json.contains("\"op\":\"input\""));
        let event: Event =
            serde_json::from_str("{\"event\":\"exited\",\"pane\":\"00000000-0000-0000-0000-000000000000\",\"clean\":true}")
                .expect("deserialize");
        assert!(matches!(event, Event::Exited { clean: true, .. }));
    }

    #[test]
    fn pane_exit_is_visible_when_its_output_stream_reaches_eof() {
        let pane = Uuid::new_v4();
        let (sender, receiver) = mpsc::channel();
        let outputs = Arc::new(Mutex::new(HashMap::from([(pane, sender)])));
        let exits = Arc::new(Mutex::new(HashMap::new()));
        let spawn_failures = Arc::new(Mutex::new(HashMap::new()));
        let observed_exits = Arc::clone(&exits);

        let observer = std::thread::spawn(move || {
            assert!(receiver.recv().is_err(), "the output stream should close");
            observed_exits.lock().expect("exits").get(&pane).copied()
        });

        finish_tracked_pane(&outputs, &exits, &spawn_failures, pane, true, None);

        assert_eq!(
            observer.join().expect("EOF observer should finish"),
            Some(true)
        );
    }
}
