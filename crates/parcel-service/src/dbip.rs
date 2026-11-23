use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use parcel_core::geo::{self, Geo};

const KINDS: [&str; 2] = ["city", "asn"];
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const BASE_URL: &str = "https://download.db-ip.com/free";

/// `PARCEL_DATA`, else `%ProgramData%\Parcel`; created if missing.
///
/// # Errors
/// When the folder can't be created.
pub fn data_dir() -> io::Result<PathBuf> {
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

/// Keeps `geo` loaded from `dir` and fetches each new month's DB-IP Lite files, until `stop`
/// is dropped. The downloads go through Parcel's own capture and show as Parcel's traffic.
pub fn keep_updated(dir: &Path, geo: &RwLock<Geo>, stop: &Receiver<()>) {
    *geo.write().expect("geo lock") = load(dir);
    loop {
        if refresh(dir) {
            *geo.write().expect("geo lock") = load(dir);
            prune(dir);
        }
        match stop.recv_timeout(CHECK_EVERY) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Spawns [`keep_updated`] on its own thread.
pub fn spawn(
    dir: PathBuf,
    geo: Arc<RwLock<Geo>>,
    stop: Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || keep_updated(&dir, &geo, &stop))
}

fn file_name(kind: &str, (year, month): (i64, u32)) -> String {
    format!("dbip-{kind}-lite-{year}-{month:02}.mmdb")
}

/// The newest file of `kind` in `dir`; names sort by month.
fn newest(dir: &Path, kind: &str) -> Option<PathBuf> {
    let prefix = format!("dbip-{kind}-lite-");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".mmdb"))
        })
        .max()
}

fn load(dir: &Path) -> Geo {
    let open = |kind: &str| {
        let path = newest(dir, kind)?;
        match geo::open(&path) {
            Ok(reader) => {
                tracing::info!("DB-IP: using {}", path.display());
                Some(reader)
            }
            Err(e) => {
                tracing::error!("DB-IP: can't open {}: {e}", path.display());
                None
            }
        }
    };
    Geo::new(open("city"), open("asn"))
}

/// Downloads this month's files when missing, falling back to last month's early in a month
/// before DB-IP publishes. Returns whether anything new arrived.
fn refresh(dir: &Path) -> bool {
    let this = month_of(SystemTime::now());
    let mut changed = false;
    for kind in KINDS {
        let have =
            newest(dir, kind).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        for month in [this, previous(this)] {
            let want = file_name(kind, month);
            if have.as_deref().is_some_and(|h| h >= want.as_str()) {
                break;
            }
            match download(dir, kind, month) {
                Ok(true) => {
                    changed = true;
                    break;
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::error!("DB-IP: downloading {want}: {e}");
                    break;
                }
            }
        }
    }
    changed
}

/// `Ok(false)` when DB-IP has no file for that month (yet).
fn download(dir: &Path, kind: &str, month: (i64, u32)) -> Result<bool, Box<dyn std::error::Error>> {
    let name = file_name(kind, month);
    let url = format!("{BASE_URL}/{name}.gz");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15 * 60)))
        .build()
        .into();
    let response = match agent.get(&url).call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    tracing::info!("DB-IP: downloading {url}");
    let part = dir.join(format!("{name}.part"));
    let result = (|| -> io::Result<()> {
        let mut out = BufWriter::new(File::create(&part)?);
        io::copy(
            &mut GzDecoder::new(response.into_body().into_reader()),
            &mut out,
        )?;
        out.into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()?;
        geo::open(&part).map_err(io::Error::other)?;
        std::fs::rename(&part, dir.join(&name))
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e.into());
    }
    tracing::info!("DB-IP: saved {name}");
    Ok(true)
}

/// Deletes all but the newest file of each kind. A file still mapped can't be deleted on
/// Windows; it goes on a later run.
fn prune(dir: &Path) {
    for kind in KINDS {
        let Some(keep) = newest(dir, kind) else {
            continue;
        };
        let prefix = format!("dbip-{kind}-lite-");
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for path in entries.filter_map(Result::ok).map(|e| e.path()) {
            let old = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
                && path != keep;
            if old && let Err(e) = std::fs::remove_file(&path) {
                tracing::debug!("DB-IP: can't delete {} yet: {e}", path.display());
            }
        }
    }
}

/// UTC (year, month) of `t`.
fn month_of(t: SystemTime) -> (i64, u32) {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, _) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    (y, m)
}

fn previous((year, month): (i64, u32)) -> (i64, u32) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

/// Days since 1970-01-01 → (year, month, day), Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_780), (2026, 11, 23));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn month_steps_back_across_years() {
        assert_eq!(previous((2027, 1)), (2026, 12));
        assert_eq!(previous((2026, 11)), (2026, 10));
    }

    #[test]
    fn file_names_sort_by_month() {
        assert_eq!(file_name("city", (2026, 3)), "dbip-city-lite-2026-03.mmdb");
        assert!(file_name("asn", (2026, 10)) > file_name("asn", (2026, 9)));
    }

    #[test]
    fn newest_picks_the_latest_month_of_a_kind() {
        let dir = std::env::temp_dir().join(format!("parcel-dbip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in [
            "dbip-city-lite-2026-09.mmdb",
            "dbip-city-lite-2026-10.mmdb",
            "dbip-city-lite-2026-11.mmdb.part",
            "dbip-asn-lite-2026-11.mmdb",
        ] {
            File::create(dir.join(n)).unwrap();
        }
        let city = newest(&dir, "city").unwrap();
        let asn = newest(&dir, "asn").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(city.ends_with("dbip-city-lite-2026-10.mmdb"), "{city:?}");
        assert!(asn.ends_with("dbip-asn-lite-2026-11.mmdb"), "{asn:?}");
    }
}
