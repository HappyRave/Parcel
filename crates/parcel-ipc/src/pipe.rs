use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, oneshot};
use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

use crate::frame::{self, Error};
use crate::messages::{ClientMsg, PROTOCOL, Reply, Request, ServerMsg, Tick};

pub const PIPE_NAME: &str = r"\\.\pipe\parcel";

/// SYSTEM and administrators get full access, interactively logged-in users read / write.
const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Answers client requests; runs on a blocking thread, so it may do I/O.
pub type Handler = Arc<dyn Fn(Request) -> Reply + Send + Sync>;

/// Accepts clients on `name` forever, streaming every tick from `ticks` to each of them and
/// answering their requests with `handler`.
///
/// # Errors
/// When the pipe can't be created, e.g. because another process already owns the name.
pub async fn serve(
    name: &str,
    ticks: broadcast::Sender<Tick>,
    service: &str,
    handler: Handler,
) -> io::Result<()> {
    let mut server = create(name, true).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            io::Error::new(
                e.kind(),
                format!("{name} is already taken: is Parcel's service running?"),
            )
        } else {
            e
        }
    })?;
    loop {
        server.connect().await?;
        let client = std::mem::replace(&mut server, create(name, false)?);
        let rx = ticks.subscribe();
        let (service, handler) = (service.to_owned(), handler.clone());
        tokio::spawn(async move {
            match handle(client, rx, service, handler).await {
                Ok(()) => tracing::debug!("client left"),
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe => {
                    tracing::debug!("client left");
                }
                Err(e) => tracing::error!("client dropped: {e}"),
            }
        });
    }
}

async fn handle(
    pipe: NamedPipeServer,
    mut ticks: broadcast::Receiver<Tick>,
    service: String,
    handler: Handler,
) -> Result<(), Error> {
    let (mut rd, mut wr) = tokio::io::split(pipe);
    let hello = tokio::time::timeout(HELLO_TIMEOUT, frame::read::<_, ClientMsg>(&mut rd))
        .await
        .map_err(|_| Error::Unexpected("no hello".into()))?;
    match hello? {
        Some(ClientMsg::Hello { protocol }) if protocol == PROTOCOL => {}
        Some(ClientMsg::Hello { protocol }) => {
            return Err(Error::Protocol {
                ours: PROTOCOL,
                theirs: protocol,
            });
        }
        Some(other) => return Err(Error::Unexpected(format!("{other:?} before hello"))),
        None => return Ok(()),
    }
    let welcome = ServerMsg::Welcome {
        protocol: PROTOCOL,
        service,
    };
    frame::write(&mut wr, &welcome).await?;

    // Reading lives in its own task: `frame::read` isn't cancel-safe inside `select!`.
    let (requests, mut incoming) = mpsc::channel(8);
    let reader = tokio::spawn(async move {
        loop {
            match frame::read::<_, ClientMsg>(&mut rd).await {
                Ok(Some(ClientMsg::Request { id, request })) => {
                    if requests.send((id, request)).await.is_err() {
                        return;
                    }
                }
                Ok(Some(ClientMsg::Hello { .. })) => tracing::warn!("second hello ignored"),
                Ok(None) => return,
                Err(e) => {
                    tracing::error!("bad client message: {e}");
                    return;
                }
            }
        }
    });
    let _stop_reader = AbortOnDrop(reader);

    loop {
        tokio::select! {
            tick = ticks.recv() => match tick {
                Ok(tick) => frame::write(&mut wr, &ServerMsg::Tick(tick)).await?,
                Err(RecvError::Lagged(n)) => tracing::warn!("client lagged, skipped {n} ticks"),
                Err(RecvError::Closed) => return Ok(()),
            },
            request = incoming.recv() => {
                let Some((id, request)) = request else {
                    return Ok(());
                };
                let h = handler.clone();
                let reply = tokio::task::spawn_blocking(move || h(request))
                    .await
                    .unwrap_or_else(|e| Reply::Error { message: format!("request failed: {e}") });
                frame::write(&mut wr, &ServerMsg::Reply { id, reply }).await?;
            }
        }
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn create(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let sddl: Vec<u16> = SDDL.encode_utf16().chain(Some(0)).collect();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; on success `sd` is a LocalAlloc'd descriptor freed below.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &raw mut sd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    // SAFETY: `attrs` and the descriptor it points to outlive the call; the kernel copies them.
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, (&raw mut attrs).cast::<c_void>())
    };
    // SAFETY: `sd` came from ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(sd) };
    server
}

type Pending = Mutex<HashMap<u32, oneshot::Sender<Reply>>>;

/// A connected, handshaken client. [`Connection::next`] yields ticks; requests go through a
/// [`Requester`], whose replies `next` routes while it reads.
pub struct Connection {
    reader: ReadHalf<NamedPipeClient>,
    requester: Requester,
    pub service: String,
}

/// Sends requests on a [`Connection`]; cheap to clone.
#[derive(Clone)]
pub struct Requester {
    writer: Arc<tokio::sync::Mutex<WriteHalf<NamedPipeClient>>>,
    pending: Arc<Pending>,
    next_id: Arc<AtomicU32>,
}

impl Connection {
    /// Opens `name` and says hello.
    ///
    /// # Errors
    /// `Io` with `NotFound` when the service isn't running; `Protocol` on a version mismatch.
    pub async fn open(name: &str) -> Result<Self, Error> {
        let pipe = loop {
            match ClientOptions::new().open(name) {
                Ok(pipe) => break pipe,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.cast_signed()) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e.into()),
            }
        };
        let (mut reader, mut writer) = tokio::io::split(pipe);
        frame::write(&mut writer, &ClientMsg::Hello { protocol: PROTOCOL }).await?;
        let service = match frame::read(&mut reader).await? {
            Some(ServerMsg::Welcome { protocol, service }) if protocol == PROTOCOL => service,
            Some(ServerMsg::Welcome { protocol, .. }) => {
                return Err(Error::Protocol {
                    ours: PROTOCOL,
                    theirs: protocol,
                });
            }
            Some(other) => return Err(Error::Unexpected(format!("{other:?}"))),
            None => return Err(Error::Unexpected("closed before welcome".into())),
        };
        Ok(Self {
            reader,
            requester: Requester {
                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                pending: Arc::default(),
                next_id: Arc::default(),
            },
            service,
        })
    }

    #[must_use]
    pub fn requester(&self) -> Requester {
        self.requester.clone()
    }

    /// The next tick, or `None` when the service closed the pipe. Replies to requests are
    /// handed to their waiting [`Requester::request`] calls on the way.
    ///
    /// # Errors
    /// On I/O failure or a malformed frame.
    pub async fn next(&mut self) -> Result<Option<Tick>, Error> {
        let result = self.read_tick().await;
        if !matches!(result, Ok(Some(_))) {
            // Dropping the senders wakes every waiting request with an error.
            self.requester.pending.lock().expect("pending lock").clear();
        }
        result
    }

    async fn read_tick(&mut self) -> Result<Option<Tick>, Error> {
        loop {
            match frame::read(&mut self.reader).await? {
                Some(ServerMsg::Tick(tick)) => return Ok(Some(tick)),
                Some(ServerMsg::Reply { id, reply }) => {
                    let waiting = self
                        .requester
                        .pending
                        .lock()
                        .expect("pending lock")
                        .remove(&id);
                    match waiting {
                        Some(tx) => {
                            let _ = tx.send(reply);
                        }
                        None => tracing::warn!("reply {id} has no request"),
                    }
                }
                Some(other) => return Err(Error::Unexpected(format!("{other:?}"))),
                None => return Ok(None),
            }
        }
    }
}

impl Requester {
    /// Sends `request` and waits for its reply. Someone must be calling
    /// [`Connection::next`] meanwhile, since that is what reads replies.
    ///
    /// # Errors
    /// On I/O failure, a timeout, or the connection closing first.
    pub async fn request(&self, request: Request) -> Result<Reply, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, tx);
        let sent = {
            let mut w = self.writer.lock().await;
            frame::write(&mut *w, &ClientMsg::Request { id, request }).await
        };
        if let Err(e) = sent {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(Error::Unexpected("the service disconnected".into())),
            Err(_) => {
                self.pending.lock().expect("pending lock").remove(&id);
                Err(Error::Unexpected("the service didn't answer".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::AppRate;

    fn test_pipe(tag: &str) -> String {
        format!(r"\\.\pipe\parcel-test-{tag}-{}", std::process::id())
    }

    fn echo_handler() -> Handler {
        Arc::new(|request| match request {
            Request::Usage { from, to } => Reply::Usage {
                apps: vec![AppRate {
                    name: format!("{from}-{to}"),
                    ..AppRate::default()
                }],
            },
        })
    }

    async fn connect(name: &str) -> Connection {
        loop {
            match Connection::open(name).await {
                Ok(conn) => return conn,
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => panic!("{e}"),
            }
        }
    }

    #[tokio::test]
    async fn client_receives_ticks() {
        let name = test_pipe("ticks");
        let (tx, _) = broadcast::channel(8);
        let server = tokio::spawn({
            let (name, tx) = (name.clone(), tx.clone());
            async move { serve(&name, tx, "test 1.0", echo_handler()).await }
        });
        let mut conn = connect(&name).await;
        assert_eq!(conn.service, "test 1.0");
        let tick = Tick {
            at: 1000,
            down: 5,
            up: 6,
            apps: Vec::new(),
        };
        tx.send(tick.clone()).unwrap();
        assert_eq!(conn.next().await.unwrap(), Some(tick));
        server.abort();
    }

    #[tokio::test]
    async fn requests_get_their_replies_between_ticks() {
        let name = test_pipe("requests");
        let (tx, _) = broadcast::channel(8);
        let server = tokio::spawn({
            let (name, tx) = (name.clone(), tx.clone());
            async move { serve(&name, tx, "test", echo_handler()).await }
        });
        let mut conn = connect(&name).await;
        let requester = conn.requester();
        let asking = tokio::spawn(async move {
            let a = requester.request(Request::Usage { from: 1, to: 2 });
            let b = requester.request(Request::Usage { from: 3, to: 4 });
            tokio::join!(a, b)
        });
        let ticking = tokio::spawn(async move {
            for at in 0..50 {
                let _ = tx.send(Tick {
                    at,
                    ..Tick::default()
                });
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        let reading = tokio::spawn(async move { while let Ok(Some(_)) = conn.next().await {} });
        let (a, b) = asking.await.unwrap();
        let name_of = |r: Reply| match r {
            Reply::Usage { apps } => apps[0].name.clone(),
            Reply::Error { message } => panic!("{message}"),
        };
        assert_eq!(name_of(a.unwrap()), "1-2");
        assert_eq!(name_of(b.unwrap()), "3-4");
        ticking.abort();
        reading.abort();
        server.abort();
    }

    #[tokio::test]
    async fn requests_fail_when_the_service_goes() {
        let name = test_pipe("gone");
        // A service that takes the request, then dies without answering.
        let mut pipe = create(&name, true).unwrap();
        let dying = tokio::spawn(async move {
            pipe.connect().await.unwrap();
            let _hello: Option<ClientMsg> = frame::read(&mut pipe).await.unwrap();
            let welcome = ServerMsg::Welcome {
                protocol: PROTOCOL,
                service: "dying".into(),
            };
            frame::write(&mut pipe, &welcome).await.unwrap();
            let request: Option<ClientMsg> = frame::read(&mut pipe).await.unwrap();
            assert!(matches!(request, Some(ClientMsg::Request { .. })));
        });
        let mut conn = connect(&name).await;
        let requester = conn.requester();
        let asking =
            tokio::spawn(async move { requester.request(Request::Usage { from: 0, to: 0 }).await });
        assert_eq!(conn.next().await.unwrap(), None);
        dying.await.unwrap();
        assert!(asking.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn second_server_on_the_same_name_fails() {
        let name = test_pipe("squat");
        let _first = create(&name, true).unwrap();
        assert!(create(&name, true).is_err());
    }
}
