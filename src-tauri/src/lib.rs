use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parcel_ipc::frame::Error;
use parcel_ipc::messages::{AppRate, Capture, Reply, Request};
use parcel_ipc::pipe::{Connection, PIPE_NAME, Requester};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::live::{Live, local_midnight};

mod live;

/// The UI's link to `parcel-service`, mirrored to the frontend as the `service` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum ServiceState {
    Connecting,
    Connected { service: String, capture: Capture },
    Offline { reason: String },
}

struct Link {
    state: Mutex<ServiceState>,
    requester: Mutex<Option<Requester>>,
    live: Mutex<Live>,
}

#[tauri::command]
fn service_state(link: State<'_, Link>) -> ServiceState {
    link.state.lock().expect("link lock").clone()
}

/// History totals per app and host between two times (ms since the epoch), busiest first.
#[tauri::command]
async fn usage(link: State<'_, Link>, from: u64, to: u64) -> Result<Vec<AppRate>, String> {
    let requester = link
        .requester
        .lock()
        .expect("link lock")
        .clone()
        .ok_or("the service isn't connected")?;
    match requester.request(Request::Usage { from, to }).await {
        Ok(Reply::Usage { apps }) => Ok(apps),
        Ok(Reply::Error { message }) => Err(message),
        Err(e) => Err(e.to_string()),
    }
}

/// Picks the app whose hosts the Live tab lists; `None` follows the busiest.
#[tauri::command]
fn select_app(app: AppHandle, path: Option<String>) {
    app.state::<Link>()
        .live
        .lock()
        .expect("live lock")
        .select(path);
    emit_live(&app);
}

fn emit_live(app: &AppHandle) {
    let view = app.state::<Link>().live.lock().expect("live lock").view();
    if let Err(e) = app.emit("live", view) {
        eprintln!("emit live: {e}");
    }
}

fn set_state(app: &AppHandle, state: ServiceState) {
    let link = app.state::<Link>();
    let mut current = link.state.lock().expect("link lock");
    if *current == state {
        return;
    }
    current.clone_from(&state);
    drop(current);
    if let Err(e) = app.emit("service", state) {
        eprintln!("emit service: {e}");
    }
}

fn set_requester(app: &AppHandle, requester: Option<Requester>) {
    *app.state::<Link>().requester.lock().expect("link lock") = requester;
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn local_offset() -> i64 {
    time::UtcOffset::current_local_offset().map_or(0, |o| i64::from(o.whole_seconds()))
}

/// Keeps the Live tab's "today" totals in step with history, once a minute.
async fn follow_today(app: AppHandle, requester: Requester) {
    loop {
        let now = now_millis();
        // History holds whole minutes; ticks after the minute's start are added live.
        let as_of = now / 60_000 * 60_000;
        let from = local_midnight(now, local_offset());
        match requester.request(Request::Usage { from, to: now }).await {
            Ok(Reply::Usage { apps }) => {
                let down = apps.iter().map(|a| a.down).sum();
                let up = apps.iter().map(|a| a.up).sum();
                app.state::<Link>()
                    .live
                    .lock()
                    .expect("live lock")
                    .set_today(down, up, as_of);
            }
            Ok(Reply::Error { message }) => eprintln!("today's totals: {message}"),
            Err(e) => {
                eprintln!("today's totals: {e}");
                return;
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// Stays connected to the service for the app's lifetime, sending the Live view every second.
async fn keep_linked(app: AppHandle) {
    loop {
        match Connection::open(PIPE_NAME).await {
            Ok(mut conn) => {
                let greeting = std::mem::take(&mut conn.greeting);
                {
                    let link = app.state::<Link>();
                    let mut live = link.live.lock().expect("live lock");
                    live.reset(greeting.capture.npcap);
                    for tick in greeting.recent {
                        live.push(tick);
                    }
                }
                set_requester(&app, Some(conn.requester()));
                set_state(
                    &app,
                    ServiceState::Connected {
                        service: greeting.service,
                        capture: greeting.capture,
                    },
                );
                emit_live(&app);
                let today = tokio::spawn(follow_today(app.clone(), conn.requester()));
                let reason = loop {
                    match conn.next().await {
                        Ok(Some(tick)) => {
                            app.state::<Link>()
                                .live
                                .lock()
                                .expect("live lock")
                                .push(tick);
                            emit_live(&app);
                        }
                        Ok(None) => break "the service stopped".to_owned(),
                        Err(e) => break e.to_string(),
                    }
                };
                today.abort();
                set_requester(&app, None);
                set_state(&app, ServiceState::Offline { reason });
            }
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                set_state(
                    &app,
                    ServiceState::Offline {
                        reason: "the service isn't running".to_owned(),
                    },
                );
            }
            Err(e) => set_state(
                &app,
                ServiceState::Offline {
                    reason: e.to_string(),
                },
            ),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Starts the Parcel app.
///
/// # Panics
/// When Tauri fails to start.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Link {
            state: Mutex::new(ServiceState::Connecting),
            requester: Mutex::new(None),
            live: Mutex::new(Live::default()),
        })
        .setup(|app| {
            tauri::async_runtime::spawn(keep_linked(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![service_state, usage, select_app])
        .run(tauri::generate_context!())
        .expect("error while running Parcel");
}
