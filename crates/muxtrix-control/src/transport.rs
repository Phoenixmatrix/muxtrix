use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerNonblockingMode, ListenerOptions, Name, Stream,
    ToFsName as _, ToNsName as _,
    traits::{Listener as _, Stream as _},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ControlRequest, ControlResponse};

const ENDPOINT_OVERRIDE: &str = "MUXTRIX_CONTROL_ENDPOINT";
const CONTROL_REGISTRY_OVERRIDE: &str = "MUXTRIX_CONTROL_REGISTRY";

pub type ControlNotifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    address: String,
    namespaced: bool,
}

impl Endpoint {
    pub fn discover() -> Result<Self, ControlError> {
        Self::discover_for_pane(None)
    }

    pub fn discover_for_pane(pane_id: Option<&str>) -> Result<Self, ControlError> {
        let endpoint_override = std::env::var_os(ENDPOINT_OVERRIDE)
            .map(|address| Self::platform(address.to_string_lossy().into_owned()));
        let routes = active_control_routes(&control_registry_directory())?;
        if let Some(pane_id) = pane_id
            && let Some(endpoint) = registered_endpoint_for_pane(&routes, pane_id)?
        {
            return Ok(endpoint);
        }
        if let Some(endpoint) = endpoint_override {
            return Ok(endpoint);
        }
        sole_registered_endpoint(&routes)?.ok_or(ControlError::NoActiveWindow)
    }

    pub fn for_instance(instance: &str) -> Result<Self, ControlError> {
        if instance.is_empty()
            || !instance.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(ControlError::InvalidInstanceName(instance.into()));
        }

        #[cfg(unix)]
        {
            Ok(Self {
                address: control_runtime_directory()?
                    .join(format!("muxtrix-{instance}.sock"))
                    .to_string_lossy()
                    .into_owned(),
                namespaced: false,
            })
        }

        #[cfg(windows)]
        {
            Ok(Self {
                address: format!("{}-{instance}", user_endpoint_suffix()),
                namespaced: true,
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = instance;
            Err(ControlError::UnsupportedPlatform)
        }
    }

    #[must_use]
    pub fn platform(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            namespaced: cfg!(windows),
        }
    }

    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            address: path.into().to_string_lossy().into_owned(),
            namespaced: false,
        }
    }

    /// Returns the exact value child processes should use to reach this endpoint.
    #[must_use]
    pub fn environment_value(&self) -> &str {
        &self.address
    }

    fn name(&self) -> Result<Name<'_>, ControlError> {
        if self.namespaced {
            self.address
                .as_str()
                .to_ns_name::<GenericNamespaced>()
                .map_err(ControlError::Io)
        } else {
            self.address
                .as_str()
                .to_fs_name::<GenericFilePath>()
                .map_err(ControlError::Io)
        }
    }
}

#[cfg(unix)]
fn control_runtime_directory() -> Result<PathBuf, ControlError> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
    let directory = runtime_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(user_endpoint_suffix()));
    std::fs::create_dir_all(&directory)?;
    if runtime_dir.is_none() {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct ControlRouteRecord {
    endpoint: String,
    process_id: u32,
    panes: Vec<String>,
}

struct ControlRegistration {
    path: PathBuf,
    record: Mutex<ControlRouteRecord>,
}

impl ControlRegistration {
    fn create(endpoint: &Endpoint, directory: &Path) -> io::Result<Self> {
        create_private_directory(directory)?;
        let record = ControlRouteRecord {
            endpoint: endpoint.environment_value().into(),
            process_id: std::process::id(),
            panes: Vec::new(),
        };
        let path = control_route_path(directory, &record.endpoint);
        write_control_route(&path, &record)?;
        Ok(Self {
            path,
            record: Mutex::new(record),
        })
    }

    fn publish_panes(&self, panes: impl IntoIterator<Item = String>) -> io::Result<()> {
        let mut panes: Vec<String> = panes.into_iter().collect();
        panes.sort_unstable();
        panes.dedup();
        let mut record = self
            .record
            .lock()
            .map_err(|_| io::Error::other("control route lock poisoned"))?;
        if record.panes == panes && self.path.exists() {
            return Ok(());
        }
        let updated = ControlRouteRecord {
            panes,
            ..record.clone()
        };
        write_control_route(&self.path, &updated)?;
        *record = updated;
        Ok(())
    }
}

impl Drop for ControlRegistration {
    fn drop(&mut self) {
        let expected = self
            .record
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        let _ = remove_control_route_if_unchanged(&self.path, expected);
    }
}

fn control_registry_directory() -> PathBuf {
    if let Some(directory) = std::env::var_os(CONTROL_REGISTRY_OVERRIDE) {
        return directory.into();
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(user_endpoint_suffix()))
        .join(".muxtrix")
        .join("control")
}

fn create_private_directory(directory: &Path) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn control_route_path(directory: &Path, endpoint: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    endpoint.hash(&mut hasher);
    directory.join(format!("{:016x}.json", hasher.finish()))
}

fn write_control_route(path: &Path, record: &ControlRouteRecord) -> io::Result<()> {
    let contents = serde_json::to_vec(record).map_err(io::Error::other)?;
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn remove_control_route_if_unchanged(
    path: &Path,
    expected: &ControlRouteRecord,
) -> io::Result<bool> {
    let current = match std::fs::read(path) {
        Ok(contents) => serde_json::from_slice::<ControlRouteRecord>(&contents).ok(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    if current.as_ref() != Some(expected) {
        return Ok(false);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn active_control_routes(directory: &Path) -> Result<Vec<ControlRouteRecord>, ControlError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ControlError::Io(error)),
    };
    let mut routes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(contents) = std::fs::read(&path) else {
            continue;
        };
        let Ok(route) = serde_json::from_slice::<ControlRouteRecord>(&contents) else {
            continue;
        };
        let endpoint = Endpoint::platform(route.endpoint.clone());
        let live = endpoint
            .name()
            .ok()
            .and_then(|name| Stream::connect(name).ok())
            .is_some();
        if live {
            routes.push(route);
        } else {
            let _ = remove_control_route_if_unchanged(&path, &route);
        }
    }
    Ok(routes)
}

fn registered_endpoint_for_pane(
    routes: &[ControlRouteRecord],
    pane_id: &str,
) -> Result<Option<Endpoint>, ControlError> {
    let mut matching = routes
        .iter()
        .filter(|route| route.panes.iter().any(|pane| pane == pane_id));
    let Some(route) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(ControlError::AmbiguousWindows(routes.len()));
    }
    Ok(Some(Endpoint::platform(route.endpoint.clone())))
}

fn sole_registered_endpoint(
    routes: &[ControlRouteRecord],
) -> Result<Option<Endpoint>, ControlError> {
    match routes {
        [] => Ok(None),
        [route] => Ok(Some(Endpoint::platform(route.endpoint.clone()))),
        _ => Err(ControlError::AmbiguousWindows(routes.len())),
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn socket_identity(endpoint: &Endpoint) -> io::Result<SocketIdentity> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(&endpoint.address)?;
    Ok(SocketIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
fn remove_socket_if_unchanged(endpoint: &Endpoint, expected: SocketIdentity) -> io::Result<bool> {
    match socket_identity(endpoint) {
        Ok(current) if current == expected => match std::fs::remove_file(&endpoint.address) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error),
        },
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

/// Removes a Unix socket only while the path still names the listener that
/// created this guard. Another process may replace a socket path without
/// waking its old listener; unconditional name reclamation would then delete
/// the replacement's endpoint during old-server teardown.
#[cfg(unix)]
struct OwnedSocketPath {
    endpoint: Endpoint,
    identity: SocketIdentity,
}

#[cfg(unix)]
impl Drop for OwnedSocketPath {
    fn drop(&mut self) {
        let _ = remove_socket_if_unchanged(&self.endpoint, self.identity);
    }
}

fn user_endpoint_suffix() -> String {
    let identity = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| "default".into());
    let mut hasher = DefaultHasher::new();
    identity.hash(&mut hasher);
    format!("muxtrix-{:016x}", hasher.finish())
}

pub struct IncomingRequest {
    pub request: ControlRequest,
    response: SyncSender<ControlResponse>,
}

impl IncomingRequest {
    pub fn respond(self, response: ControlResponse) {
        let _ = self.response.send(response);
    }
}

pub struct ControlServer {
    receiver: Receiver<IncomingRequest>,
    endpoint: Endpoint,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    registration: Option<ControlRegistration>,
}

impl ControlServer {
    pub fn bind(endpoint: Endpoint) -> Result<Self, ControlError> {
        Self::bind_inner(endpoint, None, None)
    }

    pub fn bind_with_notifier(
        endpoint: Endpoint,
        notifier: ControlNotifier,
    ) -> Result<Self, ControlError> {
        Self::bind_inner(endpoint, Some(notifier), Some(control_registry_directory()))
    }

    fn bind_inner(
        endpoint: Endpoint,
        notifier: Option<ControlNotifier>,
        registration_directory: Option<PathBuf>,
    ) -> Result<Self, ControlError> {
        let create_listener = || -> Result<_, ControlError> {
            ListenerOptions::new()
                .name(endpoint.name()?)
                .nonblocking(ListenerNonblockingMode::Accept)
                .create_sync()
                .map_err(ControlError::Io)
        };
        let listener = match create_listener() {
            Ok(listener) => listener,
            #[cfg(unix)]
            Err(ControlError::Io(bind_error)) if bind_error.kind() == io::ErrorKind::AddrInUse => {
                let occupied_identity = socket_identity(&endpoint).ok();
                // Connecting is the ownership probe. A listener that accepts
                // but never answers may be overloaded, not stale, so protocol
                // response time must never authorize unlinking its endpoint.
                match Stream::connect(endpoint.name()?) {
                    Ok(stream) => {
                        drop(stream);
                        return Err(ControlError::Io(bind_error));
                    }
                    Err(connect_error)
                        if connect_error.kind() == io::ErrorKind::ConnectionRefused =>
                    {
                        let Some(occupied_identity) = occupied_identity else {
                            return Err(ControlError::Io(bind_error));
                        };
                        match remove_socket_if_unchanged(&endpoint, occupied_identity) {
                            Ok(true) => create_listener()?,
                            Ok(false) => return Err(ControlError::Io(bind_error)),
                            Err(error) => return Err(ControlError::Io(error)),
                        }
                    }
                    // The endpoint disappeared after bind reported it occupied.
                    // Retry normally; a concurrent live owner wins with
                    // AddrInUse and is never overwritten.
                    Err(connect_error) if connect_error.kind() == io::ErrorKind::NotFound => {
                        create_listener()?
                    }
                    Err(_) => return Err(ControlError::Io(bind_error)),
                }
            }
            Err(error) => return Err(error),
        };
        #[cfg(unix)]
        let mut listener = listener;

        #[cfg(unix)]
        let socket_path = {
            let identity = socket_identity(&endpoint)?;
            listener.do_not_reclaim_name_on_drop();
            OwnedSocketPath {
                endpoint: endpoint.clone(),
                identity,
            }
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&endpoint.address, std::fs::Permissions::from_mode(0o600))?;
        }

        let registration = registration_directory
            .as_deref()
            .map(|directory| ControlRegistration::create(&endpoint, directory))
            .transpose()?;

        let (sender, receiver) = mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let thread = thread::Builder::new()
            .name("muxtrix-control".into())
            .spawn(move || {
                #[cfg(unix)]
                let _socket_path = socket_path;
                while thread_running.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok(stream) => {
                            if !thread_running.load(Ordering::Acquire) {
                                break;
                            }
                            let sender = sender.clone();
                            let notifier = notifier.clone();
                            let _ = thread::Builder::new()
                                .name("muxtrix-control-client".into())
                                .spawn(move || handle_connection(stream, sender, notifier));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::park_timeout(Duration::from_millis(50));
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
            })?;

        Ok(Self {
            receiver,
            endpoint,
            running,
            thread: Some(thread),
            registration,
        })
    }

    pub fn publish_panes(
        &self,
        panes: impl IntoIterator<Item = String>,
    ) -> Result<(), ControlError> {
        if let Some(registration) = &self.registration {
            registration.publish_panes(panes)?;
        }
        Ok(())
    }

    pub fn try_recv(&self) -> Result<IncomingRequest, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }

    /// Returns the exact endpoint value to propagate across process boundaries.
    #[must_use]
    pub fn endpoint_environment_value(&self) -> &str {
        self.endpoint.environment_value()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            // The listener is nonblocking, so unparking the idle accept loop is
            // enough to stop it. This does not depend on the endpoint pathname
            // still referring to this server.
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn handle_connection(
    mut stream: Stream,
    sender: mpsc::Sender<IncomingRequest>,
    notifier: Option<ControlNotifier>,
) {
    let _ = stream.set_recv_timeout(Some(Duration::from_secs(3)));
    let mut line = String::new();
    let request = {
        let mut reader = BufReader::new(&mut stream);
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => match serde_json::from_str::<ControlRequest>(&line) {
                Ok(request) => request,
                Err(error) => {
                    let _ = write_response(
                        &mut stream,
                        &ControlResponse::error(format!("invalid request: {error}")),
                    );
                    return;
                }
            },
        }
    };
    // Lifecycle callbacks are fire-and-forget: the agent's hook is blocked
    // for as long as this reply takes, and the hook client discards the
    // reply anyway. Acknowledge as soon as the request is queued rather
    // than after the UI thread has drained it, so a busy or stalled window
    // can never turn a hook into a timeout the agent reports to the user.
    let acknowledge_on_queue = matches!(
        request,
        ControlRequest::AgentEvent { .. } | ControlRequest::ClaudeHook { .. }
    );
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    if sender
        .send(IncomingRequest {
            request,
            response: response_sender,
        })
        .is_err()
    {
        return;
    }
    if let Some(notifier) = notifier {
        notifier();
    }
    let response = if acknowledge_on_queue {
        ControlResponse::success("lifecycle event queued")
    } else {
        response_receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|_| ControlResponse::error("Muxtrix did not answer in time"))
    };
    let _ = write_response(&mut stream, &response);
}

fn write_response(stream: &mut Stream, response: &ControlResponse) -> Result<(), ControlError> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

pub fn send_request(
    endpoint: &Endpoint,
    request: &ControlRequest,
) -> Result<ControlResponse, ControlError> {
    let mut stream = Stream::connect(endpoint.name()?)?;
    // Windows named pipes reject socket receive timeouts. Configure the pipe
    // before sending: a fast server may close its end immediately after replying.
    stream.set_nonblocking(true)?;
    let mut request = serde_json::to_vec(request)?;
    request.push(b'\n');
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut remaining = request.as_slice();
    while !remaining.is_empty() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Muxtrix control request timed out",
            )
            .into());
        }
        match stream.write(remaining) {
            #[cfg(windows)]
            Ok(0) => thread::sleep(Duration::from_millis(10)),
            #[cfg(not(windows))]
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
            Ok(written) => remaining = &remaining[written..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    read_response(
        &mut stream,
        deadline.saturating_duration_since(Instant::now()),
    )
}

fn read_response(
    reader: &mut impl Read,
    timeout: Duration,
) -> Result<ControlResponse, ControlError> {
    let deadline = Instant::now() + timeout;
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Muxtrix control response timed out",
            )
            .into());
        }
        match reader.read(&mut buffer) {
            // A PIPE_NOWAIT byte pipe can complete an empty read while its
            // peer is still connected. The deadline also bounds a closed pipe.
            #[cfg(windows)]
            Ok(0) => thread::sleep(Duration::from_millis(10)),
            #[cfg(not(windows))]
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Muxtrix closed before completing its response",
                )
                .into());
            }
            Ok(read) => {
                if let Some(end) = buffer[..read].iter().position(|byte| *byte == b'\n') {
                    response.extend_from_slice(&buffer[..end]);
                    return Ok(serde_json::from_slice(&response)?);
                }
                response.extend_from_slice(&buffer[..read]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(
                    Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("local control I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("local control JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("local control transport is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("no active Muxtrix window is registered")]
    NoActiveWindow,
    #[error(
        "{0} Muxtrix windows are active; run muxtrixctl inside the target pane or set MUXTRIX_CONTROL_ENDPOINT"
    )]
    AmbiguousWindows(usize),
    #[error("invalid local control instance name {0:?}")]
    InvalidInstanceName(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_endpoint(label: &str) -> Endpoint {
        if cfg!(windows) {
            Endpoint::platform(format!("muxtrix-test-{label}-{}", std::process::id()))
        } else {
            Endpoint::file(std::env::temp_dir().join(format!(
                "muxtrix-control-test-{label}-{}-{:?}.sock",
                std::process::id(),
                std::thread::current().id()
            )))
        }
    }

    fn test_registry(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "muxtrix-control-registry-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn response_read_preserves_fragments_across_would_block() {
        struct Fragmented {
            stage: usize,
        }
        impl Read for Fragmented {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.stage += 1;
                let fragment: &[u8] = match self.stage {
                    1 => b"{\"ok\":true,",
                    2 => return Err(io::ErrorKind::WouldBlock.into()),
                    3 => b"\"message\":\"pong\"}\n",
                    _ => return Ok(0),
                };
                buffer[..fragment.len()].copy_from_slice(fragment);
                Ok(fragment.len())
            }
        }
        let response = read_response(&mut Fragmented { stage: 0 }, Duration::from_secs(1))
            .expect("fragmented response");
        assert!(response.ok);
        assert_eq!(response.message.as_deref(), Some("pong"));
    }

    #[test]
    fn response_read_times_out_without_a_reply() {
        struct Unresponsive;
        impl Read for Unresponsive {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
        let error =
            read_response(&mut Unresponsive, Duration::from_millis(20)).expect_err("must time out");
        assert!(
            matches!(error, ControlError::Io(error) if error.kind() == io::ErrorKind::TimedOut)
        );
    }

    #[test]
    fn local_transport_sends_requests_larger_than_the_pipe_buffer() {
        let endpoint = test_endpoint("large-request");
        let server = ControlServer::bind(endpoint.clone()).expect("server should bind");
        let request = ControlRequest::SendText {
            text: "x".repeat(256 * 1024),
            pane_id: None,
        };
        let expected = request.clone();
        let handler = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let incoming = loop {
                if let Ok(incoming) = server.try_recv() {
                    break incoming;
                }
                assert!(Instant::now() < deadline, "request should arrive");
                std::thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(incoming.request, expected);
            incoming.respond(ControlResponse::success("received"));
            server
        });
        let response = send_request(&endpoint, &request).expect("large request should work");
        assert!(response.ok);
        handler.join().expect("handler should finish");
    }

    #[test]
    fn local_transport_round_trips_typed_requests() {
        let endpoint = test_endpoint("round-trip");
        let server = ControlServer::bind(endpoint.clone()).expect("server should bind");
        let handler = std::thread::spawn(move || {
            let incoming = loop {
                if let Ok(incoming) = server.try_recv() {
                    break incoming;
                }
                std::thread::yield_now();
            };
            assert_eq!(incoming.request, ControlRequest::Ping);
            incoming.respond(ControlResponse::success("pong"));
            // Keep the listener alive until the client has read the reply.
            server
        });

        let response = send_request(&endpoint, &ControlRequest::Ping).expect("request should work");
        assert!(response.ok);
        assert_eq!(response.message.as_deref(), Some("pong"));
        handler.join().expect("handler should finish");
    }

    #[cfg(unix)]
    #[test]
    fn connected_but_unresponsive_endpoint_is_never_reclaimed() {
        use std::os::unix::net::UnixListener;

        let endpoint = test_endpoint("live-owner");
        let path = PathBuf::from(endpoint.environment_value());
        let owner = UnixListener::bind(&path).expect("live owner should bind");
        let owner_identity =
            socket_identity(&endpoint).expect("live endpoint should have identity");

        let error = match ControlServer::bind(endpoint.clone()) {
            Ok(server) => {
                drop(server);
                panic!("a connected endpoint must never be overwritten");
            }
            Err(ControlError::Io(error)) => error,
            Err(error) => panic!("unexpected bind error: {error}"),
        };

        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(
            socket_identity(&endpoint).expect("live endpoint should remain"),
            owner_identity
        );
        drop(owner);
        std::fs::remove_file(path).expect("test endpoint should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn connection_refused_endpoint_is_reclaimed() {
        use std::os::unix::net::UnixListener;

        let endpoint = test_endpoint("stale-owner");
        let path = PathBuf::from(endpoint.environment_value());
        let stale = UnixListener::bind(&path).expect("stale owner should bind");
        drop(stale);

        let server = ControlServer::bind(endpoint).expect("stale endpoint should be reclaimed");
        drop(server);

        assert!(!path.exists(), "server teardown should remove its socket");
    }

    #[cfg(unix)]
    #[test]
    fn dropping_server_ignores_a_replaced_socket_path() {
        use std::os::unix::net::UnixListener;

        let endpoint = test_endpoint("replaced-owner");
        let path = PathBuf::from(endpoint.environment_value());
        let server = ControlServer::bind(endpoint.clone()).expect("server should bind");
        std::fs::remove_file(&path).expect("original endpoint should be replaceable");
        let replacement = UnixListener::bind(&path).expect("replacement should bind");
        let replacement_identity =
            socket_identity(&endpoint).expect("replacement should have identity");
        let (finished_sender, finished_receiver) = mpsc::sync_channel(1);

        let teardown = std::thread::spawn(move || {
            drop(server);
            let _ = finished_sender.send(());
        });

        finished_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("server teardown must not wait for a pathname wake-up");
        teardown.join().expect("server teardown should finish");
        assert_eq!(
            socket_identity(&endpoint).expect("replacement endpoint should remain"),
            replacement_identity
        );
        drop(replacement);
        std::fs::remove_file(path).expect("test endpoint should be removable");
    }

    #[test]
    fn concurrent_windows_route_requests_by_pane() {
        let registry = test_registry("concurrent-windows");
        let _ = std::fs::remove_dir_all(&registry);
        let first_endpoint = test_endpoint("window-one");
        let second_endpoint = test_endpoint("window-two");
        let notifier: ControlNotifier = Arc::new(|| {});
        let first = ControlServer::bind_inner(
            first_endpoint.clone(),
            Some(Arc::clone(&notifier)),
            Some(registry.clone()),
        )
        .expect("first window should bind");
        let second = ControlServer::bind_inner(
            second_endpoint.clone(),
            Some(notifier),
            Some(registry.clone()),
        )
        .expect("second window should bind");
        first
            .publish_panes(["pane-one".into()])
            .expect("first route should publish");
        second
            .publish_panes(["pane-two".into()])
            .expect("second route should publish");

        let routes = active_control_routes(&registry).expect("routes should be readable");
        assert_eq!(routes.len(), 2);
        assert_eq!(
            registered_endpoint_for_pane(&routes, "pane-one")
                .expect("pane route should be unambiguous")
                .expect("pane route should exist")
                .environment_value(),
            first_endpoint.environment_value()
        );
        assert_eq!(
            registered_endpoint_for_pane(&routes, "pane-two")
                .expect("pane route should be unambiguous")
                .expect("pane route should exist")
                .environment_value(),
            second_endpoint.environment_value()
        );
        assert!(matches!(
            sole_registered_endpoint(&routes),
            Err(ControlError::AmbiguousWindows(2))
        ));

        drop(first);
        drop(second);
        assert!(
            active_control_routes(&registry)
                .expect("empty registry should be readable")
                .is_empty()
        );
        std::fs::remove_dir(registry).expect("test registry should be removable");
    }

    #[test]
    fn instance_endpoints_are_stable_and_distinct() {
        let first = Endpoint::for_instance("window-one").expect("instance name should be valid");
        let first_again =
            Endpoint::for_instance("window-one").expect("instance name should remain valid");
        let second = Endpoint::for_instance("window-two").expect("instance name should be valid");

        assert_eq!(first, first_again);
        assert_ne!(first, second);
    }

    #[test]
    fn endpoint_exposes_a_stable_child_process_override() {
        let endpoint = Endpoint::platform("muxtrix-explicit-endpoint");
        assert_eq!(endpoint.environment_value(), "muxtrix-explicit-endpoint");
    }
}
