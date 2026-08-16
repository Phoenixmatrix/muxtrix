//! The session daemon: owns PTYs, buffers their output, serves one client.
//! Runs inside the `muxtrix` binary under `--sessiond` so packages ship no
//! extra executable.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use interprocess::local_socket::{
    ListenerOptions, SendHalf, Stream,
    traits::{Listener as _, Stream as _},
};
use muxtrix_platform::{LaunchPlan, PtySession, PtySize};
use uuid::Uuid;

use crate::{BACKLOG_LIMIT, Event, PaneSummary, Request, SessionRecord, socket_name};

struct Pane {
    session: PtySession,
    backlog: Vec<u8>,
    exited: Option<bool>,
    /// Which incarnation of this pane id the entry belongs to. A pane
    /// restart kills the pane and spawns its replacement under the same
    /// durable id, and the outgoing PTY's reader can still be running when
    /// it does — ConPTY readers in particular do not end at the kill. The
    /// generation is how that reader knows the pane it is holding the lock
    /// on is no longer its own.
    generation: u64,
}

struct Shared {
    id: Uuid,
    endpoint: String,
    panes: Mutex<HashMap<Uuid, Pane>>,
    client: Mutex<Option<SendHalf>>,
    layout: Mutex<Option<String>>,
    name: Mutex<String>,
    /// A daemon that never spawned anything must not self-terminate while
    /// the GUI is still setting up its first pane.
    ever_spawned: std::sync::atomic::AtomicBool,
    /// True once the current connection sent Attach — socket probes that
    /// connect and vanish never mark the session as taken.
    client_attached: std::sync::atomic::AtomicBool,
    /// Hands every spawn a distinct [`Pane::generation`].
    next_generation: std::sync::atomic::AtomicU64,
}

impl Shared {
    /// A session whose every pane has exited has nothing left to persist.
    /// Once no client is attached either, the daemon is pure residue — it
    /// removes its registry record and leaves, so closed sessions never
    /// pile up in the process list or block binary updates.
    fn exit_if_settled(&self) {
        if !self.ever_spawned.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        if self.client.lock().expect("client").is_some() {
            return;
        }
        let settled = self
            .panes
            .lock()
            .expect("panes")
            .values()
            .all(|pane| pane.exited.is_some());
        if !settled {
            return;
        }
        crate::remove_session_record(self.id);
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.endpoint);
        }
        // In-crate tests run the daemon on a thread; exiting would take the
        // test harness down with it. The registry removal above is the
        // observable signal either way.
        #[cfg(not(test))]
        std::process::exit(0);
    }

    fn emit(&self, event: &Event) {
        let mut guard = self.client.lock().expect("client");
        if let Some(stream) = guard.as_mut() {
            let mut line = match serde_json::to_string(event) {
                Ok(line) => line,
                Err(_) => return,
            };
            line.push('\n');
            if stream.write_all(line.as_bytes()).is_err() {
                *guard = None;
            }
        }
    }
}

fn write_registry(record: &SessionRecord) {
    if let (Some(dir), Some(path)) = (crate::sessions_directory(), crate::registry_path(record.id))
    {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string_pretty(record) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Runs the daemon until shutdown. Blocks the calling thread.
pub fn run(id: Uuid, name: String, endpoint: String) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(parent) = std::path::Path::new(&endpoint)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create session socket directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let listener = ListenerOptions::new()
        .name(socket_name(&endpoint).map_err(|error| error.to_string())?)
        // A stale socket file from a crashed daemon must not block rebinding.
        .try_overwrite(true)
        .create_sync()
        .map_err(|error| error.to_string())?;
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let shared = Arc::new(Shared {
        id,
        endpoint: endpoint.clone(),
        panes: Mutex::new(HashMap::new()),
        client: Mutex::new(None),
        layout: Mutex::new(None),
        name: Mutex::new(name),
        ever_spawned: std::sync::atomic::AtomicBool::new(false),
        client_attached: std::sync::atomic::AtomicBool::new(false),
        next_generation: std::sync::atomic::AtomicU64::new(1),
    });
    let record = || SessionRecord {
        id,
        name: shared.name.lock().expect("name").clone(),
        endpoint: endpoint.clone(),
        process_id: std::process::id(),
        created_unix: created,
        layout: shared.layout.lock().expect("layout").clone(),
        attached: shared
            .client_attached
            .load(std::sync::atomic::Ordering::Acquire),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    write_registry(&record());

    // ConPTY readers never EOF while the pseudo-console lives, so reader
    // threads alone cannot see a Windows child die — the same reason the
    // in-process session polls try_wait on Windows. This reaper covers
    // every platform; on unix the reader's EOF usually wins the race and
    // the exited-is-none guard keeps the two paths from double-emitting.
    {
        let shared = Arc::clone(&shared);
        let _ = std::thread::Builder::new()
            .name("muxtrix-daemon-reaper".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let mut emissions = Vec::new();
                    {
                        let mut panes = shared.panes.lock().expect("panes");
                        for (pane, state) in panes.iter_mut() {
                            if state.exited.is_none()
                                && let Ok(Some(status)) = state.session.try_wait()
                            {
                                let clean = status.success();
                                state.exited = Some(clean);
                                emissions.push((*pane, clean));
                            }
                        }
                    }
                    for (pane, clean) in emissions {
                        shared.emit(&Event::Exited { pane, clean });
                    }
                    shared.exit_if_settled();
                }
            });
    }

    while let Ok(stream) = listener.accept() {
        let (read_half, writer) = Stream::split(stream);
        *shared.client.lock().expect("client") = Some(writer);
        let reader = BufReader::new(read_half);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let Ok(request) = serde_json::from_str::<Request>(&line) else {
                continue;
            };
            match request {
                Request::Attach => {
                    shared
                        .client_attached
                        .store(true, std::sync::atomic::Ordering::Release);
                    write_registry(&record());
                    let panes_guard = shared.panes.lock().expect("panes");
                    let summaries: Vec<PaneSummary> = panes_guard
                        .iter()
                        .map(|(pane, state)| PaneSummary {
                            pane: *pane,
                            exited: state.exited,
                        })
                        .collect();
                    let backlogs: Vec<(Uuid, String)> = panes_guard
                        .iter()
                        .map(|(pane, state)| (*pane, BASE64.encode(&state.backlog)))
                        .collect();
                    drop(panes_guard);
                    shared.emit(&Event::Attached {
                        panes: summaries,
                        layout: shared.layout.lock().expect("layout").clone(),
                    });
                    for (pane, data) in backlogs {
                        shared.emit(&Event::Backlog { pane, data });
                        shared.emit(&Event::BacklogDone { pane });
                    }
                }
                Request::Spawn {
                    pane,
                    executable,
                    arguments,
                    working_directory,
                    environment,
                    rows,
                    cols,
                } => {
                    let plan = LaunchPlan {
                        executable,
                        arguments,
                        working_directory,
                        environment,
                    };
                    let size = PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    };
                    match PtySession::spawn(&plan, size) {
                        Ok(mut session) => {
                            let process_id = session.process_id();
                            match session.take_reader() {
                                Ok(reader) => {
                                    shared
                                        .ever_spawned
                                        .store(true, std::sync::atomic::Ordering::Release);
                                    let generation = shared
                                        .next_generation
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    // Published before its reader starts, so
                                    // the reader always finds its own entry.
                                    shared.panes.lock().expect("panes").insert(
                                        pane,
                                        Pane {
                                            session,
                                            backlog: Vec::new(),
                                            exited: None,
                                            generation,
                                        },
                                    );
                                    spawn_pane_reader(
                                        pane,
                                        generation,
                                        reader,
                                        Arc::clone(&shared),
                                    );
                                    shared.emit(&Event::Spawned { pane, process_id });
                                }
                                Err(error) => shared.emit(&Event::SpawnFailed {
                                    pane,
                                    error: error.to_string(),
                                }),
                            }
                        }
                        Err(error) => shared.emit(&Event::SpawnFailed {
                            pane,
                            error: error.to_string(),
                        }),
                    }
                }
                Request::Input { pane, data } => {
                    if let Ok(bytes) = BASE64.decode(&data)
                        && let Some(state) = shared.panes.lock().expect("panes").get_mut(&pane)
                    {
                        let _ = state.session.write_all(&bytes);
                    }
                }
                Request::Resize { pane, rows, cols } => {
                    if let Some(state) = shared.panes.lock().expect("panes").get_mut(&pane) {
                        let _ = state.session.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                }
                Request::Kill { pane } => {
                    // No Exited event follows: a killed pane is one the client
                    // asked to be rid of, and it releases the pane's byte
                    // channel itself. Announcing the death here would race a
                    // relaunch, which reuses the pane's identity — the event
                    // would land on the session that replaced this one.
                    let mut panes = shared.panes.lock().expect("panes");
                    if let Some(state) = panes.get_mut(&pane) {
                        let _ = state.session.kill();
                    }
                    panes.remove(&pane);
                }
                Request::Layout { data } => {
                    *shared.layout.lock().expect("layout") = Some(data);
                    write_registry(&record());
                }
                Request::Rename { name } => {
                    *shared.name.lock().expect("name") = name;
                    write_registry(&record());
                }
                Request::Detach => break,
                Request::Shutdown => {
                    let mut panes = shared.panes.lock().expect("panes");
                    for state in panes.values_mut() {
                        let _ = state.session.kill();
                    }
                    panes.clear();
                    crate::remove_session_record(id);
                    #[cfg(unix)]
                    {
                        let _ = std::fs::remove_file(&endpoint);
                    }
                    return Ok(());
                }
            }
        }
        // Client detached; keep serving the next attach — unless nothing
        // alive remains to serve.
        *shared.client.lock().expect("client") = None;
        if shared
            .client_attached
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            write_registry(&record());
        }
        shared.exit_if_settled();
    }
    Ok(())
}

fn spawn_pane_reader(
    pane: Uuid,
    generation: u64,
    mut reader: Box<dyn Read + Send>,
    shared: Arc<Shared>,
) {
    let _ = std::thread::Builder::new()
        .name(format!("muxtrix-daemon-pane-{pane}"))
        .spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let bytes = &buffer[..count];
                        {
                            let mut panes = shared.panes.lock().expect("panes");
                            // A replacement pane already owns this id: these
                            // bytes belong to the incarnation being replaced,
                            // and nothing may attribute them to the new one.
                            let Some(state) = panes
                                .get_mut(&pane)
                                .filter(|state| state.generation == generation)
                            else {
                                return;
                            };
                            state.backlog.extend_from_slice(bytes);
                            if state.backlog.len() > BACKLOG_LIMIT {
                                let excess = state.backlog.len() - BACKLOG_LIMIT;
                                state.backlog.drain(..excess);
                                // Replay must never start mid-escape-
                                // sequence — that renders as literal
                                // garbage. Dropping to the next line
                                // start lands on a clean boundary.
                                if let Some(newline) = state
                                    .backlog
                                    .iter()
                                    .take(8 * 1024)
                                    .position(|&byte| byte == b'\n')
                                {
                                    state.backlog.drain(..=newline);
                                }
                            }
                        }
                        shared.emit(&Event::Output {
                            pane,
                            data: BASE64.encode(bytes),
                        });
                    }
                }
            }
            // EOF: harvest the exit status with the same brief retry the
            // in-process path uses. A pane id now owned by a replacement is
            // not this reader's to report on: announcing an exit for it would
            // close the byte channel the live replacement reads through.
            let clean = {
                let mut panes = shared.panes.lock().expect("panes");
                panes
                    .get_mut(&pane)
                    .filter(|state| state.generation == generation)
                    .and_then(|state| {
                        if state.exited.is_some() {
                            // The reaper already saw and announced this exit.
                            return None;
                        }
                        let mut clean = false;
                        for _ in 0..20 {
                            match state.session.try_wait() {
                                Ok(Some(status)) => {
                                    clean = status.success();
                                    break;
                                }
                                Ok(None) => {
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }
                                Err(_) => break,
                            }
                        }
                        state.exited = Some(clean);
                        Some(clean)
                    })
            };
            if let Some(clean) = clean {
                shared.emit(&Event::Exited { pane, clean });
            }
            shared.exit_if_settled();
        });
}

/// Detached daemon spawn: the GUI calls this once per new session.
pub fn spawn_detached(id: Uuid, name: &str, endpoint: &str) -> std::io::Result<()> {
    let executable = std::env::current_exe()?;
    // Windows locks a running executable's file, and this daemon outlives
    // the GUI — run it from a copy under ~/.muxtrix/bin so the install
    // directory stays updatable.
    #[cfg(windows)]
    let executable = stage_daemon_binary(&executable).unwrap_or(executable);
    let mut command = std::process::Command::new(executable);
    command
        .arg("--sessiond")
        .arg("--session-id")
        .arg(id.to_string())
        .arg("--session-name")
        .arg(name)
        .arg("--session-endpoint")
        .arg(endpoint)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    command.spawn().map(drop)
}

/// Copies the binary into ~/.muxtrix/bin for the daemon to run from,
/// sweeping copies that no running daemon holds open any more.
#[cfg(windows)]
fn stage_daemon_binary(source: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let dir = crate::sessions_directory()
        .and_then(|sessions| Some(sessions.parent()?.join("bin")))
        .ok_or_else(|| std::io::Error::other("no home directory"))?;
    std::fs::create_dir_all(&dir)?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            // Locked copies belong to live daemons; removal simply fails.
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let destination = dir.join(format!("muxtrix-sessiond-{}.exe", std::process::id()));
    std::fs::copy(source, &destination)?;
    Ok(destination)
}

/// Waits until the daemon's socket accepts, bounded to keep startup honest.
pub fn wait_until_ready(endpoint: &str) -> bool {
    for _ in 0..100 {
        if crate::connect(endpoint).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    false
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn start_test_daemon(prefix: &str) -> (Uuid, std::path::PathBuf, crate::SessionClient) {
        let id = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("muxtrix-{prefix}-test-{id}"));
        std::fs::create_dir_all(&dir).expect("test dir");
        let endpoint = dir.join("test.sock").to_string_lossy().into_owned();
        let daemon_endpoint = endpoint.clone();
        std::thread::spawn(move || {
            let _ = run(id, "test".into(), daemon_endpoint);
        });
        assert!(wait_until_ready(&endpoint));
        let (client, _, _) = crate::SessionClient::connect_endpoint(&endpoint).expect("attach");
        (id, dir, client)
    }

    #[test]
    fn daemon_buffers_output_and_replays_it_on_reattach() {
        let id = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("muxtrix-sessiond-test-{id}"));
        let endpoint = dir
            .join("missing-socket-directory")
            .join("test.sock")
            .to_string_lossy()
            .into_owned();
        let daemon_endpoint = endpoint.clone();
        std::thread::spawn(move || {
            let _ = run(id, "test".into(), daemon_endpoint);
        });
        assert!(wait_until_ready(&endpoint), "daemon should come up");
        assert!(
            std::path::Path::new(&endpoint)
                .parent()
                .is_some_and(std::path::Path::is_dir),
            "daemon should create a missing socket directory"
        );

        let (client, panes, _) =
            crate::SessionClient::connect_endpoint(&endpoint).expect("first attach");
        assert!(panes.is_empty());
        let pane = Uuid::new_v4();
        let output = client.register_pane(pane);
        client
            .send(&Request::Spawn {
                pane,
                executable: "sh".into(),
                arguments: vec!["-c".into(), "printf persistence-marker; sleep 30".into()],
                working_directory: None,
                environment: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
            })
            .expect("spawn");
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(bytes) = output.recv_timeout(std::time::Duration::from_millis(100)) {
                seen.extend_from_slice(&bytes);
            }
            if String::from_utf8_lossy(&seen).contains("persistence-marker") {
                break;
            }
        }
        assert!(
            String::from_utf8_lossy(&seen).contains("persistence-marker"),
            "live output should stream"
        );
        drop(client); // detach — the shell keeps running

        let (client, panes, _) =
            crate::SessionClient::connect_endpoint(&endpoint).expect("reattach");
        let replay = client.register_pane(pane);
        // Backlog events race the attach reply; register again then re-attach
        // to receive the replay for the registered pane.
        client.send(&Request::Attach).expect("re-request attach");
        assert_eq!(panes.len(), 1, "the pane must survive the detach");
        let mut replayed = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(bytes) = replay.recv_timeout(std::time::Duration::from_millis(100)) {
                replayed.extend_from_slice(&bytes);
            }
            if String::from_utf8_lossy(&replayed).contains("persistence-marker") {
                break;
            }
        }
        assert!(
            String::from_utf8_lossy(&replayed).contains("persistence-marker"),
            "backlog must replay on reattach"
        );
        client.send(&Request::Shutdown).expect("shutdown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Killing a pane must release everything the daemon holds for it —
    /// its PTY, its reader and its backlog — not just stop showing it.
    #[test]
    fn killing_a_pane_releases_it_from_the_daemon() {
        let (_, dir, client) = start_test_daemon("kill");
        let pane = Uuid::new_v4();
        let output = client.register_pane(pane);
        client
            .send(&Request::Spawn {
                pane,
                executable: "sh".into(),
                arguments: vec!["-c".into(), "printf ready; sleep 300".into()],
                working_directory: None,
                environment: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
            })
            .expect("spawn");
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(bytes) = output.recv_timeout(std::time::Duration::from_millis(100)) {
                seen.extend_from_slice(&bytes);
            }
            if String::from_utf8_lossy(&seen).contains("ready") {
                break;
            }
        }
        assert!(
            String::from_utf8_lossy(&seen).contains("ready"),
            "the pane should be live before it is killed"
        );

        client.send(&Request::Kill { pane }).expect("kill");

        // Re-attaching reports what the daemon still holds; a killed pane
        // must be absent, backlog and all.
        client.send(&Request::Attach).expect("re-request attach");
        let mut reattached = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && reattached.is_none() {
            if let Ok(Event::Attached { panes, .. }) = client.try_event() {
                reattached = Some(panes);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            reattached
                .expect("the daemon should answer the re-attach")
                .len(),
            0,
            "a killed pane must be gone from the daemon, not left holding a PTY and its backlog"
        );
        client.send(&Request::Shutdown).expect("shutdown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The daemon a long-lived session runs may predate the fix above, so
    /// the client releases the pane itself rather than waiting to be told.
    #[test]
    fn unregistering_a_pane_closes_its_output_channel_without_the_daemon() {
        let (_, dir, client) = start_test_daemon("unregister");
        let pane = Uuid::new_v4();
        let output = client.register_pane(pane);
        client.unregister_pane(pane);
        assert!(
            matches!(
                output.recv_timeout(std::time::Duration::from_secs(1)),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
            ),
            "unregistering must drop the pane's output sender"
        );
        // A late exit for a pane the client has forgotten must not resurrect
        // its bookkeeping.
        assert_eq!(client.pane_exit(pane), None);
        client.send(&Request::Shutdown).expect("shutdown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Restarting a pane whose process had already died reuses its id while
    /// the client still remembers that death. The replacement must start
    /// with a clean record: on Windows the session loop asks `pane_exit`
    /// every 50 ms and would report the live replacement as exited.
    #[test]
    fn registering_a_pane_again_forgets_the_incarnation_before_it() {
        let (_, dir, client) = start_test_daemon("reregister");
        let pane = Uuid::new_v4();
        let output = client.register_pane(pane);
        client
            .send(&Request::Spawn {
                pane,
                executable: "sh".into(),
                arguments: vec!["-c".into(), "exit 3".into()],
                working_directory: None,
                environment: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
            })
            .expect("spawn");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && client.pane_exit(pane).is_none() {
            let _ = output.recv_timeout(std::time::Duration::from_millis(50));
        }
        assert_eq!(
            client.pane_exit(pane),
            Some(false),
            "the pane should have exited before it is restarted"
        );

        let _replacement = client.register_pane(pane);
        assert_eq!(
            client.pane_exit(pane),
            None,
            "a replacement pane must not inherit the exit of the pane it replaced"
        );
        assert!(!client.pane_replaying(pane));
        assert_eq!(client.pane_spawn_failure(pane), None);
        client.send(&Request::Shutdown).expect("shutdown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pane the host cannot start must say why. The spawn request itself
    /// succeeds — it is only written to a socket — so without this the
    /// refusal is invisible.
    #[test]
    fn a_refused_spawn_keeps_its_reason_for_the_pane() {
        let (_, dir, client) = start_test_daemon("spawn-failure");
        let pane = Uuid::new_v4();
        let output = client.register_pane(pane);
        client
            .send(&Request::Spawn {
                pane,
                // The shell a restart was asked for, unreachable — the same
                // shape as a WSL distribution that will not come up.
                executable: dir
                    .join("shell-that-does-not-exist")
                    .to_string_lossy()
                    .into_owned(),
                arguments: Vec::new(),
                working_directory: None,
                environment: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
            })
            .expect("the request itself is accepted");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && client.pane_spawn_failure(pane).is_none() {
            let _ = output.recv_timeout(std::time::Duration::from_millis(50));
        }
        assert!(
            client.pane_spawn_failure(pane).is_some(),
            "the pane should carry the host's refusal, not just fall silent"
        );
        client.send(&Request::Shutdown).expect("shutdown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_exits_on_its_own_once_every_pane_is_dead_and_nobody_is_attached() {
        let (id, dir, client) = start_test_daemon("settle");
        let pane = Uuid::new_v4();
        let output = client.register_pane(pane);
        client
            .send(&Request::Spawn {
                pane,
                executable: "sh".into(),
                arguments: vec!["-c".into(), "exit 0".into()],
                working_directory: None,
                environment: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
            })
            .expect("spawn");
        // Wait for the pane to die (its output channel closes).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if output
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err()
                && client.pane_exit(pane).is_some()
            {
                break;
            }
        }
        assert!(client.pane_exit(pane).is_some(), "pane should have exited");
        drop(client); // detach — nothing alive remains
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut settled = false;
        while std::time::Instant::now() < deadline {
            if crate::registry_path(id).is_none_or(|path| !path.exists()) {
                settled = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            settled,
            "a settled daemon must clean its registry record and exit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pane restart reuses the pane's durable identity: kill, then spawn
    /// again under the same id. The replacement must stream its own output —
    /// nothing the outgoing pane's reader does as it winds down may be
    /// attributed to it.
    #[test]
    fn respawning_a_killed_pane_id_keeps_the_replacement_streaming() {
        let (_, dir, client) = start_test_daemon("respawn");
        let pane = Uuid::new_v4();

        // The outgoing shell leaves a child holding the PTY open, so its
        // reader outlives the kill — a pane running an agent, a build, or
        // anything backgrounded. On Windows the same is true of every pane:
        // a ConPTY reader does not EOF when the shell is killed.
        let spawn = |marker: &str| Request::Spawn {
            pane,
            executable: "sh".into(),
            arguments: vec!["-c".into(), format!("sleep 1 & printf {marker}; sleep 300")],
            working_directory: None,
            environment: vec![("TERM".into(), "xterm-256color".into())],
            rows: 24,
            cols: 80,
        };
        let await_marker = |output: &std::sync::mpsc::Receiver<Vec<u8>>, marker: &str| {
            let mut seen = Vec::new();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while std::time::Instant::now() < deadline {
                match output.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(bytes) => seen.extend_from_slice(&bytes),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                if String::from_utf8_lossy(&seen).contains(marker) {
                    return true;
                }
            }
            false
        };

        // Several rounds: the outgoing reader's EOF has to be scheduled
        // against the replacement's spawn, and one round can win by luck.
        for round in 0..5 {
            let output = client.register_pane(pane);
            client.send(&spawn("live-marker")).expect("spawn");
            assert!(
                await_marker(&output, "live-marker"),
                "round {round}: the pane should stream before it is restarted"
            );

            // Exactly what a restart does on the client side.
            client.send(&Request::Kill { pane }).expect("kill");
            client.unregister_pane(pane);
            let replacement = client.register_pane(pane);
            client.send(&spawn("replacement-marker")).expect("respawn");
            assert!(
                await_marker(&replacement, "replacement-marker"),
                "round {round}: the replacement pane never streamed — its output \
                 channel was closed by the pane it replaced"
            );
            // The outgoing pane's reader reaches EOF around here, once its
            // lingering child lets go of the PTY.
            std::thread::sleep(std::time::Duration::from_millis(1500));
            assert_eq!(
                client.pane_exit(pane),
                None,
                "round {round}: the replacement is alive, so nothing may record an exit for it"
            );
            assert!(
                client.tracks_pane(pane),
                "round {round}: the replacement's output channel was closed by the \
                 pane it replaced — a live terminal that receives no bytes"
            );
            client.send(&Request::Kill { pane }).expect("kill");
            client.unregister_pane(pane);
        }

        client.send(&Request::Shutdown).expect("shutdown");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
