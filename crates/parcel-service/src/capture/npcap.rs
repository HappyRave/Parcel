use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use parcel_core::capture::{CaptureError, CaptureSource};
use parcel_core::event::{Event, now_millis};
use parcel_core::packet::Decoder;
use pcap::{Capture, Device};
use windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW;

use crate::adapters;

const NAME: &str = "Npcap";

/// Packet capture on every hardware adapter that is up, one thread per adapter.
#[derive(Default)]
pub struct Npcap {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

fn failed(message: impl ToString) -> CaptureError {
    CaptureError::Failed {
        source_name: NAME,
        message: message.to_string(),
    }
}

/// Points the delay-loaded `wpcap.dll` at Npcap's folder. Without this check, a missing Npcap
/// would crash the process at the first pcap call.
fn load_npcap() -> Result<(), CaptureError> {
    let system =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let dir = system.join("System32").join("Npcap");
    if !dir.join("wpcap.dll").is_file() {
        return Err(CaptureError::NotInstalled(NAME));
    }
    let wide: Vec<u16> = dir
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    if unsafe { SetDllDirectoryW(wide.as_ptr()) } == 0 {
        return Err(failed(std::io::Error::last_os_error()));
    }
    Ok(())
}

impl CaptureSource for Npcap {
    fn start(&mut self, events: Sender<Event>) -> Result<(), CaptureError> {
        load_npcap()?;
        let hardware = adapters::hardware_up().map_err(|e| failed(format!("GetIfTable2: {e}")))?;
        let devices: Vec<Device> = Device::list()
            .map_err(failed)?
            .into_iter()
            .filter(|d| hardware.iter().any(|g| d.name.ends_with(g.as_str())))
            .collect();
        if devices.is_empty() {
            return Err(failed("no network adapter to capture on"));
        }
        let local: Vec<IpAddr> = devices
            .iter()
            .flat_map(|d| d.addresses.iter().map(|a| a.addr))
            .collect();

        self.stop.store(false, Ordering::Relaxed);
        for device in devices {
            let label = device.desc.clone().unwrap_or_else(|| device.name.clone());
            let cap = Capture::from_device(device)
                .and_then(|c| c.snaplen(65535).timeout(200).buffer_size(16 << 20).open());
            let mut cap = match cap {
                Ok(cap) => cap,
                Err(e) => {
                    tracing::error!("{NAME}: can't open {label}: {e}");
                    continue;
                }
            };
            let (stop, events, local) = (self.stop.clone(), events.clone(), local.clone());
            tracing::info!("{NAME}: capturing on {label}");
            self.threads.push(std::thread::spawn(move || {
                let mut decoder = Decoder::new(local);
                let mut closed = false;
                while !stop.load(Ordering::Relaxed) && !closed {
                    match cap.next_packet() {
                        Ok(p) => decoder.decode(now_millis(), p.data, p.header.len, &mut |e| {
                            closed |= events.send(e).is_err();
                        }),
                        Err(pcap::Error::TimeoutExpired) => {}
                        Err(e) => {
                            tracing::error!("{NAME}: {label}: {e}");
                            break;
                        }
                    }
                }
            }));
        }
        if self.threads.is_empty() {
            return Err(failed("no adapter could be opened"));
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            if t.join().is_err() {
                tracing::error!("{NAME}: a capture thread panicked");
            }
        }
    }
}
