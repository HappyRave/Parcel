use std::fs::File;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

mod adapters;
mod capture;
mod data;
mod dbip;
mod history;
mod process;
mod runner;
mod service;

const USAGE: &str = "usage: parcel-service <command>
  console [--record <events.jsonl>]   run in this terminal until Ctrl+C (admin for ETW)
  install                             register and start the Parcel service (admin)
  uninstall                           stop and remove the Parcel service (admin)";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Console(Option<PathBuf>),
    Install,
    Uninstall,
    /// Started by the Service Control Manager.
    Service,
}

fn parse(args: &[String]) -> Option<Command> {
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["console"] => Some(Command::Console(None)),
        ["console", "--record", path] => Some(Command::Console(Some(PathBuf::from(path)))),
        ["install"] => Some(Command::Install),
        ["uninstall"] => Some(Command::Uninstall),
        [arg] if arg == service::RUN_ARG => Some(Command::Service),
        _ => None,
    }
}

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())
}

/// The service has no console: it logs to `parcel-service.log` in the data folder, fresh each
/// start.
fn log_to_file() -> Result<(), String> {
    let path = data::dir()
        .map_err(|e| format!("data folder: {e}"))?
        .join("parcel-service.log");
    let file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .init();
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if command == Command::Service {
        if let Err(e) = log_to_file() {
            // Nowhere else to say it; the Service Control Manager records the exit code.
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    } else {
        tracing_subscriber::fmt().with_env_filter(filter()).init();
    }
    let result = match command {
        Command::Console(record) => {
            tracing::info!("Ctrl+C to stop");
            runner::run(record.as_deref(), async {
                if let Err(e) = tokio::signal::ctrl_c().await {
                    tracing::error!("Ctrl+C handler: {e}");
                    std::future::pending::<()>().await;
                }
            })
        }
        Command::Install => service::install(),
        Command::Uninstall => service::uninstall(),
        Command::Service => service::dispatch(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse(&args("console")), Some(Command::Console(None)));
        assert_eq!(
            parse(&args("console --record e.jsonl")),
            Some(Command::Console(Some("e.jsonl".into())))
        );
        assert_eq!(parse(&args("install")), Some(Command::Install));
        assert_eq!(parse(&args("uninstall")), Some(Command::Uninstall));
        assert_eq!(parse(&args(service::RUN_ARG)), Some(Command::Service));
    }

    #[test]
    fn anything_else_is_usage() {
        for bad in ["", "consol", "console --record", "install now"] {
            assert_eq!(parse(&args(bad)), None, "{bad:?}");
        }
    }
}
