//! Session persistence: a per-session daemon owns every PTY so shells and
//! agents keep running when the GUI closes, tmux-style. The GUI attaches
//! over a local socket, replays each pane's ring-buffered backlog into a
//! fresh VT, and streams from there. One daemon process per session; a
//! JSON registry under `~/.muxtrix/sessions` lists what exists.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, SendHalf, Stream, ToFsName as _, ToNsName as _,
    traits::Stream as _,
};

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
    /// All buffered history for the pane has been replayed; output after
    /// this is live. Clients gate PTY query-response write-back on it —
    /// answering a historical query types garbage into the live shell.
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
    },
    SpawnFailed {
        pane: Uuid,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneSummary {
    pub pane: Uuid,
    pub exited: Option<bool>,
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
/// hands each pane a byte stream plus control handles.
pub struct SessionClient {
    writer: Arc<Mutex<SendHalf>>,
    events: Mutex<Receiver<Event>>,
    pane_outputs: Arc<Mutex<HashMap<Uuid, Sender<Vec<u8>>>>>,
    pane_exits: Arc<Mutex<HashMap<Uuid, bool>>>,
    pane_pids: Arc<Mutex<HashMap<Uuid, u32>>>,
    /// Panes currently replaying backlog; absent means live.
    pane_replaying: Arc<Mutex<HashMap<Uuid, bool>>>,
    /// Why the host could not start a pane's process. Without this a failed
    /// spawn is indistinguishable from a pane that simply never printed:
    /// the request was written successfully, and the failure arrives later
    /// as an event nobody reads.
    pane_spawn_failures: Arc<Mutex<HashMap<Uuid, String>>>,
}

impl SessionClient {
    pub fn connect_endpoint(
        endpoint: &str,
    ) -> std::io::Result<(Self, Vec<PaneSummary>, Option<String>)> {
        let stream = connect(endpoint)?;
        let (read_half, mut writer) = Stream::split(stream);
        let (event_tx, event_rx) = mpsc::channel();
        let pane_outputs: Arc<Mutex<HashMap<Uuid, Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pane_exits: Arc<Mutex<HashMap<Uuid, bool>>> = Arc::new(Mutex::new(HashMap::new()));
        let pane_pids: Arc<Mutex<HashMap<Uuid, u32>>> = Arc::new(Mutex::new(HashMap::new()));
        let pane_replaying: Arc<Mutex<HashMap<Uuid, bool>>> = Arc::new(Mutex::new(HashMap::new()));
        let pane_spawn_failures: Arc<Mutex<HashMap<Uuid, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let outputs = Arc::clone(&pane_outputs);
        let exits = Arc::clone(&pane_exits);
        let pids = Arc::clone(&pane_pids);
        let replaying = Arc::clone(&pane_replaying);
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
                        Event::Backlog { pane, data } => {
                            replaying.lock().expect("replaying").insert(*pane, true);
                            if let Ok(bytes) = BASE64.decode(data)
                                && let Some(sender) = outputs.lock().expect("outputs").get(pane)
                            {
                                let _ = sender.send(bytes);
                            }
                            continue;
                        }
                        Event::BacklogDone { pane } => {
                            replaying.lock().expect("replaying").remove(pane);
                            continue;
                        }
                        Event::Output { pane, data } => {
                            // Live output also ends replay — covers daemons
                            // predating BacklogDone.
                            replaying.lock().expect("replaying").remove(pane);
                            if let Ok(bytes) = BASE64.decode(data)
                                && let Some(sender) = outputs.lock().expect("outputs").get(pane)
                            {
                                let _ = sender.send(bytes);
                            }
                            continue;
                        }
                        Event::Exited { pane, clean } => {
                            // Closing the pane's output channel is the EOF
                            // its reader thread is waiting for. A pane the
                            // client already unregistered is gone for good;
                            // recording its exit would regrow the maps that
                            // unregistering just cleared.
                            let tracked = outputs.lock().expect("outputs").remove(pane).is_some();
                            if tracked {
                                exits.lock().expect("exits").insert(*pane, *clean);
                            }
                            continue;
                        }
                        Event::Spawned {
                            pane,
                            process_id: Some(process_id),
                        } => {
                            pids.lock().expect("pids").insert(*pane, *process_id);
                            continue;
                        }
                        Event::SpawnFailed { pane, error } => {
                            // Failure reads as an unclean immediate exit;
                            // dropping the sender is the reader's EOF. The
                            // reason is kept so the pane can say why it is
                            // empty instead of looking like a live terminal.
                            let tracked = outputs.lock().expect("outputs").remove(pane).is_some();
                            if tracked {
                                exits.lock().expect("exits").insert(*pane, false);
                                spawn_failures
                                    .lock()
                                    .expect("spawn failures")
                                    .insert(*pane, error.clone());
                            }
                            continue;
                        }
                        _ => {}
                    }
                    if event_tx.send(event).is_err() {
                        break;
                    }
                }
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
                pane_exits,
                pane_pids,
                pane_replaying,
                pane_spawn_failures,
            },
            panes,
            layout,
        ))
    }

    pub fn send(&self, request: &Request) -> std::io::Result<()> {
        send_line(&mut self.writer.lock().expect("session writer"), request)
    }

    pub fn try_event(&self) -> Result<Event, TryRecvError> {
        self.events.lock().expect("session events").try_recv()
    }

    /// Registers a pane and returns the receiver its output bytes arrive on.
    ///
    /// Pane ids are durable, so this is also how a replacement claims the id
    /// of the pane it replaces. Everything recorded about the previous
    /// incarnation is dropped here: an inherited exit makes the replacement
    /// report itself dead the moment it is polled, and an inherited replay
    /// flag makes it swallow the terminal's own responses.
    pub fn register_pane(&self, pane: Uuid) -> Receiver<Vec<u8>> {
        let (sender, receiver) = mpsc::channel();
        self.pane_exits.lock().expect("exits").remove(&pane);
        self.pane_pids.lock().expect("pids").remove(&pane);
        self.pane_replaying.lock().expect("replaying").remove(&pane);
        self.pane_spawn_failures
            .lock()
            .expect("spawn failures")
            .remove(&pane);
        self.pane_outputs
            .lock()
            .expect("outputs")
            .insert(pane, sender);
        receiver
    }

    /// Forgets a pane the GUI has closed. Dropping the pane's output sender
    /// is the EOF its reader thread blocks on — a pane dropped without this
    /// strands that thread, and its bookkeeping, for the life of the process.
    pub fn unregister_pane(&self, pane: Uuid) {
        self.pane_outputs.lock().expect("outputs").remove(&pane);
        self.pane_exits.lock().expect("exits").remove(&pane);
        self.pane_pids.lock().expect("pids").remove(&pane);
        self.pane_replaying.lock().expect("replaying").remove(&pane);
        self.pane_spawn_failures
            .lock()
            .expect("spawn failures")
            .remove(&pane);
    }

    /// Whether the client is still streaming this pane. False once the pane
    /// has exited or been unregistered — in both cases its byte channel is
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

    /// Whether the pane is still replaying buffered history.
    pub fn pane_replaying(&self, pane: Uuid) -> bool {
        self.pane_replaying
            .lock()
            .expect("replaying")
            .contains_key(&pane)
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
}
