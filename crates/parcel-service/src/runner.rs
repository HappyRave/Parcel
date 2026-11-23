use std::collections::VecDeque;
use std::fs::File;
use std::future::Future;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, RwLock};

use parcel_core::capture::CaptureSource;
use parcel_core::event::Event;
use parcel_core::geo::Geo;
use parcel_core::store::{Recorder, Store};
use parcel_ipc::messages::{Capture, Greeting};
use parcel_ipc::pipe::{self, Greeter, PIPE_NAME};
use tokio::sync::broadcast;

use crate::capture::etw::Etw;
use crate::capture::npcap::Npcap;
use crate::capture::ports::PortTable;
use crate::process::WinProcesses;
use crate::{data, dbip, history};

/// Seconds of ticks kept for new clients: the Live graph's 5-minute window.
const RECENT: usize = 300;

/// Captures, records and serves the app until `stop` completes. Shared by console mode
/// (Ctrl+C) and the Windows service (the Service Control Manager's stop).
///
/// # Errors
/// A message when setup fails or the pipe server stops on its own.
pub fn run(record: Option<&Path>, stop: impl Future<Output = ()>) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|e| format!("tokio runtime: {e}"))?;
    let (events, rx) = mpsc::channel::<Event>();
    let rx = match record {
        Some(path) => {
            recorder(path, rx).map_err(|e| format!("can't record to {}: {e}", path.display()))?
        }
        None => rx,
    };

    let data = data::dir().map_err(|e| format!("data folder: {e}"))?;
    let db = data.join("parcel.db");
    let (store, answer) = Store::open(&db)
        .and_then(|store| Ok((store, history::handler(&db)?)))
        .map_err(|e| format!("history database {}: {e}", db.display()))?;
    let geo = Arc::new(RwLock::new(Geo::default()));
    // Never joined: it may be mid-download at shutdown, and holds nothing that needs closing.
    let (stop_dbip, dbip_stopped) = mpsc::channel();
    dbip::spawn(data, geo.clone(), dbip_stopped);

    let mut npcap = Npcap::default();
    let mut etw = Etw::default();
    let mut ports = PortTable::default();
    let npcap_on = start(&mut npcap, &events);
    let etw_on = start(&mut etw, &events);
    start(&mut ports, &events);
    if !npcap_on && !etw_on {
        tracing::error!("nothing is capturing: install Npcap or run as administrator");
    }

    let (ticks, _) = broadcast::channel(16);
    // The live graph's per-second history, kept in memory only.
    let recent = Arc::new(Mutex::new(VecDeque::with_capacity(RECENT)));
    let engine = std::thread::spawn({
        let (ticks, recent) = (ticks.clone(), recent.clone());
        move || {
            let mut history = Recorder::new(store);
            parcel_core::engine::run(&rx, npcap_on, WinProcesses::new(), geo, |tick| {
                if let Err(e) = history.add(&tick) {
                    tracing::error!("history: a minute was lost: {e}");
                }
                // Into `recent` before broadcasting, so a client greeted in between can't miss it.
                {
                    let mut recent = recent.lock().expect("recent lock");
                    if recent.len() == RECENT {
                        recent.pop_front();
                    }
                    recent.push_back(tick.clone());
                }
                // No subscribers is normal while the UI is closed.
                let _ = ticks.send(tick);
            });
            if let Err(e) = history.flush() {
                tracing::error!("history: the last minute was lost: {e}");
            }
        }
    });

    let version = concat!("parcel-service ", env!("CARGO_PKG_VERSION"));
    tracing::info!("{version} on {PIPE_NAME}");
    let capture = Capture {
        npcap: npcap_on,
        etw: etw_on,
    };
    let greet: Greeter = Arc::new(move || Greeting {
        service: version.to_owned(),
        capture,
        recent: recent
            .lock()
            .expect("recent lock")
            .iter()
            .cloned()
            .collect(),
    });
    let served = runtime.block_on(async {
        tokio::select! {
            r = pipe::serve(PIPE_NAME, ticks, greet, answer) => r,
            () = stop => Ok(()),
        }
    });

    tracing::info!("stopping");
    drop(stop_dbip);
    npcap.stop();
    etw.stop();
    ports.stop();
    drop(events);
    if engine.join().is_err() {
        return Err("engine thread panicked".into());
    }
    served.map_err(|e| e.to_string())
}

fn start(source: &mut dyn CaptureSource, events: &mpsc::Sender<Event>) -> bool {
    match source.start(events.clone()) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("{e}");
            false
        }
    }
}

/// Writes every event to `path` as one JSON line, then passes it on.
fn recorder(path: &Path, rx: Receiver<Event>) -> std::io::Result<Receiver<Event>> {
    let mut out = BufWriter::new(File::create(path)?);
    let (fwd, fwd_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for event in rx {
            let written = serde_json::to_writer(&mut out, &event)
                .map_err(std::io::Error::from)
                .and_then(|()| out.write_all(b"\n"));
            if let Err(e) = written {
                tracing::error!("recording stopped: {e}");
                return;
            }
            if fwd.send(event).is_err() {
                break;
            }
        }
        if let Err(e) = out.flush() {
            tracing::error!("recording: {e}");
        }
    });
    tracing::info!("recording events to {}", path.display());
    Ok(fwd_rx)
}
