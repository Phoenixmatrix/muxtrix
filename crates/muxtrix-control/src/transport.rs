use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream, ToFsName as _,
    ToNsName as _,
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
        let listener = ListenerOptions::new()
            .name(endpoint.name()?)
            .try_overwrite(true)
            .create_sync()?;

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
                while let Ok(stream) = listener.accept() {
                    if !thread_running.load(Ordering::Acquire) {
                        break;
                    }
                    let sender = sender.clone();
                    let notifier = notifier.clone();
                    let _ = thread::Builder::new()
                        .name("muxtrix-control-client".into())
                        .spawn(move || handle_connection(stream, sender, notifier));
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
        if let Ok(name) = self.endpoint.name()
            && let Ok(mut stream) = Stream::connect(name)
        {
            let _ = stream.write_all(b"\n");
        }
        if let Some(thread) = self.thread.take() {
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

    #[test]
    fn local_transport_round_trips_typed_requests() {
        let endpoint = if cfg!(windows) {
            Endpoint::platform(format!("muxtrix-test-{}", std::process::id()))
        } else {
            Endpoint::file(std::env::temp_dir().join(format!(
                "muxtrix-control-test-{}-{:?}.sock",
                std::process::id(),
                std::thread::current().id()
            )))
        };
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

    #[test]
    fn endpoint_exposes_a_stable_child_process_override() {
        let endpoint = Endpoint::platform("muxtrix-explicit-endpoint");
        assert_eq!(endpoint.environment_value(), "muxtrix-explicit-endpoint");
    }
}
