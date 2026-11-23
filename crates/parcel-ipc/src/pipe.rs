use std::ffi::c_void;
use std::io;
use std::time::Duration;

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

use crate::frame::{self, Error};
use crate::messages::{ClientMsg, PROTOCOL, ServerMsg, Tick};

pub const PIPE_NAME: &str = r"\\.\pipe\parcel";

/// SYSTEM and administrators get full access, interactively logged-in users read / write.
const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Accepts clients on `name` forever, streaming every tick from `ticks` to each of them.
///
/// # Errors
/// When the pipe can't be created, e.g. because another process already owns the name.
pub async fn serve(name: &str, ticks: broadcast::Sender<Tick>, service: &str) -> io::Result<()> {
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
        let service = service.to_owned();
        tokio::spawn(async move {
            match handle(client, rx, service).await {
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
    mut pipe: NamedPipeServer,
    mut ticks: broadcast::Receiver<Tick>,
    service: String,
) -> Result<(), Error> {
    let hello = tokio::time::timeout(HELLO_TIMEOUT, frame::read::<_, ClientMsg>(&mut pipe))
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
        None => return Ok(()),
    }
    let welcome = ServerMsg::Welcome {
        protocol: PROTOCOL,
        service,
    };
    frame::write(&mut pipe, &welcome).await?;
    loop {
        match ticks.recv().await {
            Ok(tick) => frame::write(&mut pipe, &ServerMsg::Tick(tick)).await?,
            Err(RecvError::Lagged(n)) => tracing::warn!("client lagged, skipped {n} ticks"),
            Err(RecvError::Closed) => return Ok(()),
        }
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

/// A connected, handshaken client.
pub struct Connection {
    pipe: NamedPipeClient,
    pub service: String,
}

impl Connection {
    /// Opens `name` and says hello.
    ///
    /// # Errors
    /// `Io` with `NotFound` when the service isn't running; `Protocol` on a version mismatch.
    pub async fn open(name: &str) -> Result<Self, Error> {
        let mut pipe = loop {
            match ClientOptions::new().open(name) {
                Ok(pipe) => break pipe,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.cast_signed()) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e.into()),
            }
        };
        frame::write(&mut pipe, &ClientMsg::Hello { protocol: PROTOCOL }).await?;
        match frame::read(&mut pipe).await? {
            Some(ServerMsg::Welcome { protocol, service }) if protocol == PROTOCOL => {
                Ok(Self { pipe, service })
            }
            Some(ServerMsg::Welcome { protocol, .. }) => Err(Error::Protocol {
                ours: PROTOCOL,
                theirs: protocol,
            }),
            Some(other) => Err(Error::Unexpected(format!("{other:?}"))),
            None => Err(Error::Unexpected("closed before welcome".into())),
        }
    }

    /// The next message, or `None` when the service closed the pipe.
    ///
    /// # Errors
    /// On I/O failure or a malformed frame.
    pub async fn next(&mut self) -> Result<Option<ServerMsg>, Error> {
        frame::read(&mut self.pipe).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pipe(tag: &str) -> String {
        format!(r"\\.\pipe\parcel-test-{tag}-{}", std::process::id())
    }

    #[tokio::test]
    async fn client_receives_ticks() {
        let name = test_pipe("ticks");
        let (tx, _) = broadcast::channel(8);
        let server = tokio::spawn({
            let (name, tx) = (name.clone(), tx.clone());
            async move { serve(&name, tx, "test 1.0").await }
        });
        let mut conn = loop {
            match Connection::open(&name).await {
                Ok(conn) => break conn,
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => panic!("{e}"),
            }
        };
        assert_eq!(conn.service, "test 1.0");
        let tick = Tick {
            at: 1000,
            down: 5,
            up: 6,
        };
        tx.send(tick).unwrap();
        assert_eq!(conn.next().await.unwrap(), Some(ServerMsg::Tick(tick)));
        server.abort();
    }

    #[tokio::test]
    async fn second_server_on_the_same_name_fails() {
        let name = test_pipe("squat");
        let _first = create(&name, true).unwrap();
        assert!(create(&name, true).is_err());
    }
}
