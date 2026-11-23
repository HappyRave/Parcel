use std::ffi::OsString;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::runner;

pub const NAME: &str = "Parcel";
const DISPLAY_NAME: &str = "Parcel";
const DESCRIPTION: &str =
    "Records this PC's network traffic by app and remote host for the Parcel app.";
/// The argument the Service Control Manager starts us with.
pub const RUN_ARG: &str = "run";
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SERVICE_EXISTS: i32 = 1073;
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

fn explain(e: windows_service::Error, doing: &str) -> String {
    match &e {
        windows_service::Error::Winapi(io) => match io.raw_os_error() {
            Some(ERROR_ACCESS_DENIED) => format!("{doing}: run this as administrator"),
            Some(ERROR_SERVICE_EXISTS) => {
                format!("{doing}: the {NAME} service is already installed")
            }
            Some(ERROR_SERVICE_DOES_NOT_EXIST) => {
                format!("{doing}: the {NAME} service isn't installed")
            }
            _ => format!("{doing}: {io}"),
        },
        other => format!("{doing}: {other}"),
    }
}

/// Registers this executable as the `Parcel` service (automatic start, LocalSystem, restarts
/// after a crash) and starts it.
///
/// # Errors
/// A message, e.g. when not elevated or already installed.
pub fn install() -> Result<(), String> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|e| explain(e, "opening the service manager"))?;
    let exe = std::env::current_exe().map_err(|e| format!("finding this executable: {e}"))?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.clone(),
        launch_arguments: vec![RUN_ARG.into()],
        dependencies: vec![],
        account_name: None, // LocalSystem: ETW and Npcap need it
        account_password: None,
    };
    let service = manager
        .create_service(
            &info,
            ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
        )
        .map_err(|e| explain(e, "installing"))?;
    service
        .set_description(DESCRIPTION)
        .map_err(|e| explain(e, "describing"))?;
    let restart = |secs| ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(secs),
    };
    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![restart(5), restart(30), restart(300)]),
        })
        .map_err(|e| explain(e, "setting crash recovery"))?;
    // Also restart when we stop with an error exit code, not only on a crash.
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(|e| explain(e, "setting crash recovery"))?;
    service
        .start::<OsString>(&[])
        .map_err(|e| explain(e, "starting"))?;
    println!(
        "installed and started the {NAME} service ({})",
        exe.display()
    );
    Ok(())
}

/// Stops the `Parcel` service and removes it.
///
/// # Errors
/// A message, e.g. when not elevated or not installed.
pub fn uninstall() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| explain(e, "opening the service manager"))?;
    let service = manager
        .open_service(
            NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .map_err(|e| explain(e, "opening the service"))?;
    let state = service
        .query_status()
        .map_err(|e| explain(e, "querying"))?
        .current_state;
    if state != ServiceState::Stopped {
        service.stop().map_err(|e| explain(e, "stopping"))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while service
            .query_status()
            .map_err(|e| explain(e, "querying"))?
            .current_state
            != ServiceState::Stopped
        {
            if Instant::now() > deadline {
                return Err("the service didn't stop within 15 s".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    service.delete().map_err(|e| explain(e, "removing"))?;
    println!("removed the {NAME} service");
    Ok(())
}

/// Hands this process to the Service Control Manager; returns when the service stops.
///
/// # Errors
/// When not started by the Service Control Manager.
pub fn dispatch() -> Result<(), String> {
    service_dispatcher::start(NAME, ffi_service_main)
        .map_err(|e| explain(e, "running as a service"))
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = serve() {
        tracing::error!("{e}");
    }
}

fn serve() -> Result<(), String> {
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let stop = Mutex::new(Some(stop));
    let status = service_control_handler::register(NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(stop) = stop.lock().expect("stop lock").take() {
                let _ = stop.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(|e| explain(e, "registering the control handler"))?;

    let report = |state, accept, exit| {
        status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: exit,
            checkpoint: 0,
            wait_hint: Duration::from_secs(20),
            process_id: None,
        })
    };
    report(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceExitCode::Win32(0),
    )
    .map_err(|e| explain(e, "reporting running"))?;

    let result = runner::run(None, async {
        let _ = stopped.await;
    });
    let exit = match &result {
        Ok(()) => ServiceExitCode::Win32(0),
        // A non-zero exit lets the recovery actions restart us.
        Err(_) => ServiceExitCode::ServiceSpecific(1),
    };
    report(ServiceState::Stopped, ServiceControlAccept::empty(), exit)
        .map_err(|e| explain(e, "reporting stopped"))?;
    result
}
