use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerNonblockingMode, ListenerOptions, Name, Stream,
    ToFsName as _, ToNsName as _,
    traits::{Listener as _, Stream as _},
};
use thiserror::Error;

use crate::{ControlRequest, ControlResponse};

const ENDPOINT_OVERRIDE: &str = "MUXTRIX_CONTROL_ENDPOINT";

pub type ControlNotifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    address: String,
    namespaced: bool,
}

impl Endpoint {
    pub fn discover() -> Result<Self, ControlError> {
        if let Some(address) = std::env::var_os(ENDPOINT_OVERRIDE) {
            return Ok(Self::platform(address.to_string_lossy().into_owned()));
        }

        #[cfg(unix)]
        {
            let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
            let base = runtime_dir
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join(user_endpoint_suffix()));
            std::fs::create_dir_all(&base)?;
            if runtime_dir.is_none() {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(Self {
                address: base.join("muxtrix.sock").to_string_lossy().into_owned(),
                namespaced: false,
            })
        }

        #[cfg(windows)]
        {
            Ok(Self {
                address: format!("muxtrix-{}", user_endpoint_suffix()),
                namespaced: true,
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
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
}

impl ControlServer {
    pub fn bind(endpoint: Endpoint) -> Result<Self, ControlError> {
        Self::bind_inner(endpoint, None)
    }

    fn bind_inner(
        endpoint: Endpoint,
        notifier: Option<ControlNotifier>,
    ) -> Result<Self, ControlError> {
        let create_listener = || -> Result<_, ControlError> {
            ListenerOptions::new()
                .name(endpoint.name()?)
                .nonblocking(ListenerNonblockingMode::Accept)
                .create_sync()
                .map_err(ControlError::Io)
        };
        let mut listener = match create_listener() {
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
        })
    }

    pub fn discover_and_bind() -> Result<Self, ControlError> {
        Self::bind(Endpoint::discover()?)
    }

    pub fn discover_and_bind_with_notifier(
        notifier: ControlNotifier,
    ) -> Result<Self, ControlError> {
        Self::bind_inner(Endpoint::discover()?, Some(notifier))
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
    #[cfg(unix)]
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
    let response = response_receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap_or_else(|_| ControlResponse::error("Muxtrix did not answer in time"));
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
    #[cfg(unix)]
    stream.set_recv_timeout(Some(Duration::from_secs(4)))?;
    serde_json::to_writer(&mut stream, request)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("local control I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("local control JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("local control transport is unsupported on this platform")]
    UnsupportedPlatform,
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
    fn endpoint_exposes_a_stable_child_process_override() {
        let endpoint = Endpoint::platform("muxtrix-explicit-endpoint");
        assert_eq!(endpoint.environment_value(), "muxtrix-explicit-endpoint");
    }
}
