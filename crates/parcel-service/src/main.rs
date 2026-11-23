use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};

use parcel_core::capture::CaptureSource;
use parcel_core::event::Event;
use parcel_ipc::pipe::{self, PIPE_NAME};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

use crate::capture::etw::Etw;
use crate::capture::npcap::Npcap;
use crate::capture::ports::PortTable;
use crate::process::WinProcesses;

mod adapters;
mod capture;
mod process;

const USAGE: &str = "usage: parcel-service console [--record <events.jsonl>]";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["console"] => console(None),
        ["console", "--record", path] => console(Some(PathBuf::from(path))),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Runs the service in this terminal until Ctrl+C, for development.
fn console(record: Option<PathBuf>) -> ExitCode {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (events, rx) = mpsc::channel::<Event>();
    let rx = match record {
        Some(path) => match recorder(&path, rx) {
            Ok(rx) => rx,
            Err(e) => {
                tracing::error!("can't record to {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => rx,
    };

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
    let engine = std::thread::spawn({
        let ticks = ticks.clone();
        move || {
            parcel_core::engine::run(&rx, npcap_on, WinProcesses::new(), |tick| {
                // No subscribers is normal while the UI is closed.
                let _ = ticks.send(tick);
            });
        }
    });

    let version = concat!("parcel-service ", env!("CARGO_PKG_VERSION"));
    tracing::info!("{version} on {PIPE_NAME}, Ctrl+C to stop");
    let result = runtime.block_on(async {
        tokio::select! {
            r = pipe::serve(PIPE_NAME, ticks, version) => r,
            r = tokio::signal::ctrl_c() => r,
        }
    });

    npcap.stop();
    etw.stop();
    ports.stop();
    drop(events);
    if engine.join().is_err() {
        tracing::error!("engine thread panicked");
        return ExitCode::FAILURE;
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
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
fn recorder(path: &PathBuf, rx: Receiver<Event>) -> std::io::Result<Receiver<Event>> {
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
