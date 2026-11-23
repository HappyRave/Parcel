use std::process::ExitCode;
use std::sync::mpsc;

use parcel_core::event::Event;
use parcel_ipc::pipe::{self, PIPE_NAME};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: parcel-service console";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    match std::env::args().nth(1).as_deref() {
        Some("console") => console(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Runs the service in this terminal until Ctrl+C, for development.
fn console() -> ExitCode {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (ticks, _) = broadcast::channel(16);
    let (events, rx) = mpsc::channel::<Event>();
    let engine = std::thread::spawn({
        let ticks = ticks.clone();
        move || {
            parcel_core::engine::run(&rx, false, |tick| {
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
