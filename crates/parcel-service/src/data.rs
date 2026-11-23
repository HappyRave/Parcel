use std::io;
use std::path::PathBuf;

/// `PARCEL_DATA`, else `%ProgramData%\Parcel`; created if missing. Holds the history database,
/// the DB-IP files and the service log.
///
/// # Errors
/// When the folder can't be created.
pub fn dir() -> io::Result<PathBuf> {
    let dir = match std::env::var_os("PARCEL_DATA") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(
            std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into()),
        )
        .join("Parcel"),
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
