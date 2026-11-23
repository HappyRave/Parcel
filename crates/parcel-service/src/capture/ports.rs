use std::ffi::c_void;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use parcel_core::capture::{CaptureError, CaptureSource};
use parcel_core::event::{Event, PortOwner, Proto, now_millis};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

const EVERY: Duration = Duration::from_secs(1);

/// Polls the OS socket tables so packets are attributed even before ETW sees the flow, and at
/// all when ETW can't run (no admin).
#[derive(Default)]
pub struct PortTable {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureSource for PortTable {
    fn start(&mut self, events: Sender<Event>) -> Result<(), CaptureError> {
        let (stop, stopped) = channel();
        self.stop = Some(stop);
        self.thread = Some(std::thread::spawn(move || {
            loop {
                let ports = Event::Ports {
                    at: now_millis(),
                    owners: owners(),
                };
                if events.send(ports).is_err() {
                    return;
                }
                match stopped.recv_timeout(EVERY) {
                    Err(RecvTimeoutError::Timeout) => {}
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        }));
        Ok(())
    }

    fn stop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("port table thread panicked");
        }
    }
}

fn owners() -> Vec<PortOwner> {
    let tcp = |af: u16| {
        // SAFETY: `fetch` passes a buffer of at least `size` bytes, or null to ask for the size.
        move |p, s| unsafe {
            GetExtendedTcpTable(p, s, 0, u32::from(af), TCP_TABLE_OWNER_PID_ALL, 0)
        }
    };
    let udp = |af: u16| {
        // SAFETY: as above.
        move |p, s| unsafe { GetExtendedUdpTable(p, s, 0, u32::from(af), UDP_TABLE_OWNER_PID, 0) }
    };
    let own = |proto, port: u32, pid| PortOwner {
        proto,
        port: port_from(port),
        pid,
    };
    let mut out = Vec::new();
    out.extend(
        read_table::<MIB_TCPROW_OWNER_PID>(tcp(AF_INET))
            .iter()
            .map(|r| own(Proto::Tcp, r.dwLocalPort, r.dwOwningPid)),
    );
    out.extend(
        read_table::<MIB_TCP6ROW_OWNER_PID>(tcp(AF_INET6))
            .iter()
            .map(|r| own(Proto::Tcp, r.dwLocalPort, r.dwOwningPid)),
    );
    out.extend(
        read_table::<MIB_UDPROW_OWNER_PID>(udp(AF_INET))
            .iter()
            .map(|r| own(Proto::Udp, r.dwLocalPort, r.dwOwningPid)),
    );
    out.extend(
        read_table::<MIB_UDP6ROW_OWNER_PID>(udp(AF_INET6))
            .iter()
            .map(|r| own(Proto::Udp, r.dwLocalPort, r.dwOwningPid)),
    );
    out
}

/// The port sits in the low 16 bits, in network byte order.
fn port_from(dw: u32) -> u16 {
    u16::from_be((dw & 0xFFFF) as u16)
}

/// Reads a `{ count: u32, rows: [R] }` table, growing the buffer if the table grew meanwhile.
fn read_table<R: Copy>(fetch: impl Fn(*mut c_void, *mut u32) -> u32) -> Vec<R> {
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    let mut size = 0u32;
    fetch(std::ptr::null_mut(), &raw mut size);
    for _ in 0..3 {
        // u64 storage keeps the rows aligned.
        let mut buf = vec![0u64; (size as usize).div_ceil(8) + 1];
        match fetch(buf.as_mut_ptr().cast(), &raw mut size) {
            0 => {
                // SAFETY: on success the buffer holds the count, then `count` rows at the first
                // offset aligned for `R` (at least 4, after the count).
                return unsafe {
                    let count = *buf.as_ptr().cast::<u32>() as usize;
                    let rows = buf
                        .as_ptr()
                        .cast::<u8>()
                        .add(align_of::<R>().max(4))
                        .cast::<R>();
                    std::slice::from_raw_parts(rows, count).to_vec()
                };
            }
            ERROR_INSUFFICIENT_BUFFER => {}
            err => {
                tracing::error!("socket table read failed: {err}");
                return Vec::new();
            }
        }
    }
    tracing::error!("socket table kept growing");
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_is_network_order_in_the_low_word() {
        assert_eq!(port_from(0xBB01), 443);
        assert_eq!(port_from(0xFFFF_3500), 53);
    }

    #[test]
    fn reads_live_tables() {
        let owners = owners();
        assert!(!owners.is_empty(), "no sockets at all?");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mine = owners_with(port);
        assert_eq!(mine, Some(std::process::id()), "port {port}");
    }

    fn owners_with(port: u16) -> Option<u32> {
        owners()
            .into_iter()
            .find(|o| o.proto == Proto::Tcp && o.port == port)
            .map(|o| o.pid)
    }
}
