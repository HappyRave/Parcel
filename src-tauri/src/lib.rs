use std::sync::Mutex;
use std::time::Duration;

use parcel_ipc::frame::Error;
use parcel_ipc::messages::ServerMsg;
use parcel_ipc::pipe::{Connection, PIPE_NAME};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// The UI's link to `parcel-service`, mirrored to the frontend as the `service` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum ServiceState {
    Connecting,
    Connected { service: String },
    Offline { reason: String },
}

struct Link(Mutex<ServiceState>);

#[tauri::command]
fn service_state(link: State<'_, Link>) -> ServiceState {
    link.0.lock().expect("link lock").clone()
}

fn set_state(app: &AppHandle, state: ServiceState) {
    let link = app.state::<Link>();
    let mut current = link.0.lock().expect("link lock");
    if *current == state {
        return;
    }
    current.clone_from(&state);
    drop(current);
    if let Err(e) = app.emit("service", state) {
        eprintln!("emit service: {e}");
    }
}

/// Stays connected to the service for the app's lifetime, forwarding ticks as `tick` events.
async fn keep_linked(app: AppHandle) {
    loop {
        match Connection::open(PIPE_NAME).await {
            Ok(mut conn) => {
                set_state(
                    &app,
                    ServiceState::Connected {
                        service: conn.service.clone(),
                    },
                );
                let reason = loop {
                    match conn.next().await {
                        Ok(Some(ServerMsg::Tick(tick))) => {
                            if let Err(e) = app.emit("tick", tick) {
                                eprintln!("emit tick: {e}");
                            }
                        }
                        Ok(Some(other)) => break format!("unexpected message: {other:?}"),
                        Ok(None) => break "the service stopped".to_owned(),
                        Err(e) => break e.to_string(),
                    }
                };
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
        .manage(Link(Mutex::new(ServiceState::Connecting)))
        .setup(|app| {
            tauri::async_runtime::spawn(keep_linked(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![service_state])
        .run(tauri::generate_context!())
        .expect("error while running Parcel");
}
