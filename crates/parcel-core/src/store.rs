use std::collections::HashMap;
use std::path::Path;

use parcel_ipc::messages::{AppRate, HostRate, Tick};
use rusqlite::{Connection, OpenFlags, params};

/// Minute rows are kept this long, hour rows `HOUR_KEEP`; day rows forever.
const MINUTE_KEEP: i64 = 24 * 60;
const HOUR_KEEP: i64 = 30 * 24;

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 3_600_000;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS apps (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS hosts (
    id INTEGER PRIMARY KEY,
    host TEXT NOT NULL UNIQUE,
    service TEXT, domain TEXT, country TEXT, city TEXT, asn INTEGER, network TEXT
);
CREATE TABLE IF NOT EXISTS minute (
    t INTEGER NOT NULL, app INTEGER NOT NULL, host INTEGER NOT NULL,
    down INTEGER NOT NULL, up INTEGER NOT NULL, pdown INTEGER NOT NULL, pup INTEGER NOT NULL,
    PRIMARY KEY (t, app, host)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS hour (
    t INTEGER NOT NULL, app INTEGER NOT NULL, host INTEGER NOT NULL,
    down INTEGER NOT NULL, up INTEGER NOT NULL, pdown INTEGER NOT NULL, pup INTEGER NOT NULL,
    PRIMARY KEY (t, app, host)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS day (
    t INTEGER NOT NULL, app INTEGER NOT NULL, host INTEGER NOT NULL,
    down INTEGER NOT NULL, up INTEGER NOT NULL, pdown INTEGER NOT NULL, pup INTEGER NOT NULL,
    PRIMARY KEY (t, app, host)
) WITHOUT ROWID;
";

/// The local UTC offset in seconds right now; UTC when the OS won't say.
#[must_use]
pub fn local_offset() -> i64 {
    time::UtcOffset::current_local_offset().map_or(0, |o| i64::from(o.whole_seconds()))
}

/// Local calendar day (days since 1970-01-01) of `at_ms`, for a UTC offset in seconds.
#[must_use]
pub fn day_key(at_ms: u64, offset: i64) -> i64 {
    (i64::try_from(at_ms / 1000).unwrap_or(i64::MAX) + offset).div_euclid(86_400)
}

fn key(at_ms: u64, unit_ms: u64) -> i64 {
    i64::try_from(at_ms / unit_ms).unwrap_or(i64::MAX)
}

fn int(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// History in SQLite: traffic totals per app and remote host, per minute (24 h), hour (30 days)
/// and local day (forever). Never individual packets.
pub struct Store {
    db: Connection,
}

impl Store {
    /// Opens (creating if needed) the database the service writes.
    ///
    /// # Errors
    /// When the file can't be opened or its schema created.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// Opens an existing database read-only, for answering queries beside the writer.
    ///
    /// # Errors
    /// When the file can't be opened.
    pub fn open_read(path: &Path) -> rusqlite::Result<Self> {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self { db })
    }

    /// # Errors
    /// Never in practice; SQLite's in-memory database.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> rusqlite::Result<Self> {
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    /// Adds one minute's totals (`apps`, all ending in minute `at_ms`) to every resolution.
    ///
    /// # Errors
    /// On any SQLite failure; nothing of the minute is written then.
    pub fn add(&mut self, at_ms: u64, offset: i64, apps: &[AppRate]) -> rusqlite::Result<()> {
        let keys = [
            ("minute", key(at_ms, MINUTE_MS)),
            ("hour", key(at_ms, HOUR_MS)),
            ("day", day_key(at_ms, offset)),
        ];
        let tx = self.db.transaction()?;
        {
            let mut app_id = tx.prepare_cached(
                "INSERT INTO apps (path, name) VALUES (?1, ?2)
                 ON CONFLICT (path) DO UPDATE SET name = excluded.name RETURNING id",
            )?;
            let mut host_id = tx.prepare_cached(
                "INSERT INTO hosts (host, service, domain, country, city, asn, network)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (host) DO UPDATE SET service = excluded.service,
                   domain = excluded.domain, country = excluded.country, city = excluded.city,
                   asn = excluded.asn, network = excluded.network
                 RETURNING id",
            )?;
            for app in apps {
                let a: i64 = app_id.query_row(params![app.path, app.name], |r| r.get(0))?;
                for h in &app.hosts {
                    let hid: i64 = host_id.query_row(
                        params![
                            h.host, h.service, h.domain, h.country, h.city, h.asn, h.network
                        ],
                        |r| r.get(0),
                    )?;
                    for (table, t) in keys {
                        tx.prepare_cached(&format!(
                            "INSERT INTO {table} (t, app, host, down, up, pdown, pup)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                             ON CONFLICT (t, app, host) DO UPDATE SET
                               down = down + excluded.down, up = up + excluded.up,
                               pdown = pdown + excluded.pdown, pup = pup + excluded.pup"
                        ))?
                        .execute(params![
                            t,
                            a,
                            hid,
                            int(h.down),
                            int(h.up),
                            int(h.packets_down),
                            int(h.packets_up)
                        ])?;
                    }
                }
            }
        }
        tx.commit()
    }

    /// Drops minute rows older than 24 h and hour rows older than 30 days, as of `now_ms`.
    ///
    /// # Errors
    /// On any SQLite failure.
    pub fn prune(&self, now_ms: u64) -> rusqlite::Result<()> {
        self.db.execute(
            "DELETE FROM minute WHERE t < ?1",
            [key(now_ms, MINUTE_MS) - MINUTE_KEEP],
        )?;
        self.db.execute(
            "DELETE FROM hour WHERE t < ?1",
            [key(now_ms, HOUR_MS) - HOUR_KEEP],
        )?;
        Ok(())
    }

    /// Totals per app and host from `from_ms` to `to_ms`, busiest first, read from the finest
    /// resolution still kept for `from_ms` (so edges snap to minutes, hours or local days).
    ///
    /// # Errors
    /// On any SQLite failure.
    pub fn usage(
        &self,
        from_ms: u64,
        to_ms: u64,
        now_ms: u64,
        offset: i64,
    ) -> rusqlite::Result<Vec<AppRate>> {
        let age = now_ms.saturating_sub(from_ms);
        let (table, from, to) = if age <= MINUTE_KEEP.unsigned_abs() * MINUTE_MS {
            ("minute", key(from_ms, MINUTE_MS), key(to_ms, MINUTE_MS))
        } else if age <= HOUR_KEEP.unsigned_abs() * HOUR_MS {
            ("hour", key(from_ms, HOUR_MS), key(to_ms, HOUR_MS))
        } else {
            ("day", day_key(from_ms, offset), day_key(to_ms, offset))
        };
        let mut stmt = self.db.prepare_cached(&format!(
            "SELECT a.path, a.name, h.host, h.service, h.domain, h.country, h.city, h.asn,
                    h.network, SUM(t.down), SUM(t.up), SUM(t.pdown), SUM(t.pup)
             FROM {table} t JOIN apps a ON a.id = t.app JOIN hosts h ON h.id = t.host
             WHERE t.t BETWEEN ?1 AND ?2
             GROUP BY t.app, t.host"
        ))?;
        let mut apps: HashMap<String, AppRate> = HashMap::new();
        let rows = stmt.query_map([from, to], |r| {
            let path: String = r.get(0)?;
            let name: String = r.get(1)?;
            let n = |i| r.get::<_, i64>(i).map(i64::unsigned_abs);
            let host = HostRate {
                host: r.get(2)?,
                service: r.get(3)?,
                domain: r.get(4)?,
                country: r.get(5)?,
                city: r.get(6)?,
                asn: r.get(7)?,
                network: r.get(8)?,
                down: n(9)?,
                up: n(10)?,
                packets_down: n(11)?,
                packets_up: n(12)?,
            };
            Ok((path, name, host))
        })?;
        for row in rows {
            let (path, name, host) = row?;
            let app = apps.entry(path.clone()).or_insert_with(|| AppRate {
                path,
                name,
                ..AppRate::default()
            });
            app.down += host.down;
            app.up += host.up;
            app.packets_down += host.packets_down;
            app.packets_up += host.packets_up;
            app.hosts.push(host);
        }
        let mut apps: Vec<AppRate> = apps.into_values().collect();
        for a in &mut apps {
            a.hosts.sort_by(|x, y| {
                (y.down + y.up)
                    .cmp(&(x.down + x.up))
                    .then(x.host.cmp(&y.host))
            });
        }
        apps.sort_by(|x, y| {
            (y.down + y.up)
                .cmp(&(x.down + x.up))
                .then(x.name.cmp(&y.name))
        });
        Ok(apps)
    }
}

/// Folds per-second ticks into the current minute and writes each minute when it ends.
pub struct Recorder {
    store: Store,
    /// Minute key and the merged traffic of the minute being collected.
    minute: Option<(i64, u64, Vec<AppRate>)>,
    pruned_hour: i64,
}

impl Recorder {
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self {
            store,
            minute: None,
            pruned_hour: 0,
        }
    }

    /// Adds one tick; writes the previous minute when this tick starts a new one.
    ///
    /// # Errors
    /// When writing a finished minute fails; that minute is dropped.
    pub fn add(&mut self, tick: &Tick) -> rusqlite::Result<()> {
        // A tick stamped 12:00:00.000 covers 11:59:59 to 12:00:00, so it belongs to 11:59.
        let at = tick.at.saturating_sub(1);
        let minute = key(at, MINUTE_MS);
        let mut result = Ok(());
        if self.minute.as_ref().is_some_and(|(m, ..)| *m != minute) {
            result = self.flush();
        }
        let (_, last_at, apps) = self.minute.get_or_insert_with(|| (minute, at, Vec::new()));
        *last_at = at;
        merge(apps, &tick.apps);
        result
    }

    /// Writes the minute collected so far (call on shutdown too).
    ///
    /// # Errors
    /// On any SQLite failure; the minute is dropped.
    pub fn flush(&mut self) -> rusqlite::Result<()> {
        let Some((_, at, apps)) = self.minute.take() else {
            return Ok(());
        };
        if !apps.is_empty() {
            self.store.add(at, local_offset(), &apps)?;
        }
        let hour = key(at, HOUR_MS);
        if hour != self.pruned_hour {
            self.store.prune(at)?;
            self.pruned_hour = hour;
        }
        Ok(())
    }
}

/// Adds `more` into `into`, matching apps by path and hosts by name; newer metadata wins.
fn merge(into: &mut Vec<AppRate>, more: &[AppRate]) {
    for app in more {
        let pos = into.iter().position(|a| a.path == app.path);
        let a = match pos {
            Some(i) => &mut into[i],
            None => {
                into.push(AppRate {
                    path: app.path.clone(),
                    ..AppRate::default()
                });
                into.last_mut().expect("just pushed")
            }
        };
        a.name.clone_from(&app.name);
        for h in &app.hosts {
            match a.hosts.iter_mut().find(|x| x.host == h.host) {
                Some(x) => {
                    let (down, up, pd, pu) = (x.down, x.up, x.packets_down, x.packets_up);
                    x.clone_from(h);
                    x.down += down;
                    x.up += up;
                    x.packets_down += pd;
                    x.packets_up += pu;
                }
                None => a.hosts.push(h.clone()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_795_440_000_000; // 2026-11-23 13:20:00 UTC
    const DAY_MS: u64 = 86_400_000;

    fn host(name: &str, down: u64, up: u64) -> HostRate {
        HostRate {
            host: name.into(),
            service: Some(name.into()),
            down,
            up,
            packets_down: 1,
            packets_up: 1,
            ..HostRate::default()
        }
    }

    fn app(path: &str, hosts: Vec<HostRate>) -> AppRate {
        AppRate {
            path: path.into(),
            name: path.to_uppercase(),
            down: hosts.iter().map(|h| h.down).sum(),
            up: hosts.iter().map(|h| h.up).sum(),
            hosts,
            ..AppRate::default()
        }
    }

    fn tick(at: u64, apps: Vec<AppRate>) -> Tick {
        Tick {
            at,
            apps,
            ..Tick::default()
        }
    }

    /// Per app: name, down, then per host: name, down, up.
    type Summary = Vec<(String, u64, Vec<(String, u64, u64)>)>;

    fn summary(apps: &[AppRate]) -> Summary {
        apps.iter()
            .map(|a| {
                let hosts = a
                    .hosts
                    .iter()
                    .map(|h| (h.host.clone(), h.down, h.up))
                    .collect();
                (a.name.clone(), a.down, hosts)
            })
            .collect()
    }

    #[test]
    fn local_days_follow_the_offset() {
        // 23:30 UTC is already the next day at UTC+1, still the same day at UTC-5.
        let late = 1_795_390_200_000; // 2026-11-22 23:30 UTC
        let utc = day_key(late, 0);
        assert_eq!(day_key(late, 3600), utc + 1);
        assert_eq!(day_key(late, -5 * 3600), utc);
    }

    #[test]
    fn minutes_add_into_every_resolution() {
        let mut s = Store::open_in_memory().unwrap();
        s.add(T0, 0, &[app("a", vec![host("x.com", 100, 10)])])
            .unwrap();
        s.add(T0 + MINUTE_MS, 0, &[app("a", vec![host("x.com", 50, 5)])])
            .unwrap();
        for table in ["minute", "hour", "day"] {
            let (rows, down): (i64, i64) =
                s.db.query_row(
                    &format!("SELECT COUNT(*), SUM(down) FROM {table}"),
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            let want_rows = if table == "minute" { 2 } else { 1 };
            assert_eq!((rows, down), (want_rows, 150), "{table}");
        }
    }

    #[test]
    fn usage_sums_per_app_and_host_busiest_first() {
        let mut s = Store::open_in_memory().unwrap();
        s.add(
            T0,
            0,
            &[
                app("a", vec![host("x.com", 100, 0), host("y.com", 5, 0)]),
                app("b", vec![host("x.com", 500, 0)]),
            ],
        )
        .unwrap();
        s.add(T0 + MINUTE_MS, 0, &[app("a", vec![host("y.com", 200, 0)])])
            .unwrap();
        let usage = s
            .usage(T0 - HOUR_MS, T0 + HOUR_MS, T0 + MINUTE_MS, 0)
            .unwrap();
        assert_eq!(
            summary(&usage),
            vec![
                ("B".into(), 500, vec![("x.com".into(), 500, 0)]),
                (
                    "A".into(),
                    305,
                    vec![("y.com".into(), 205, 0), ("x.com".into(), 100, 0)]
                ),
            ]
        );
    }

    #[test]
    fn usage_reads_the_resolution_that_still_covers_the_range() {
        let mut s = Store::open_in_memory().unwrap();
        s.add(T0, 0, &[app("a", vec![host("x.com", 100, 0)])])
            .unwrap();
        let later = T0 + 2 * DAY_MS;
        s.prune(later).unwrap();
        // Minute rows are gone after 24 h; a 30-day range still finds the hour rows.
        assert!(
            s.usage(later - HOUR_MS, later, later, 0)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            s.usage(later - 30 * DAY_MS, later, later, 0).unwrap()[0].down,
            100
        );
        // Past 30 days only day rows remain.
        let much_later = T0 + 40 * DAY_MS;
        s.prune(much_later).unwrap();
        assert_eq!(s.usage(0, much_later, much_later, 0).unwrap()[0].down, 100);
    }

    #[test]
    fn host_metadata_follows_the_latest_minute() {
        let mut s = Store::open_in_memory().unwrap();
        s.add(T0, 0, &[app("a", vec![host("x.com", 1, 0)])])
            .unwrap();
        let mut renamed = host("x.com", 1, 0);
        renamed.service = Some("X".into());
        s.add(T0 + MINUTE_MS, 0, &[app("a", vec![renamed])])
            .unwrap();
        let usage = s.usage(T0 - HOUR_MS, T0 + HOUR_MS, T0, 0).unwrap();
        assert_eq!(usage[0].hosts[0].service.as_deref(), Some("X"));
        assert_eq!(usage[0].hosts[0].down, 2);
    }

    #[test]
    fn recorder_writes_each_minute_when_the_next_begins() {
        let mut r = Recorder::new(Store::open_in_memory().unwrap());
        let one = || vec![app("a", vec![host("x.com", 10, 1)])];
        for s in 1..=60 {
            r.add(&tick(T0 + s * 1000, one())).unwrap();
        }
        let count = |r: &Recorder| -> i64 {
            r.store
                .db
                .query_row("SELECT COUNT(*) FROM minute", [], |x| x.get(0))
                .unwrap()
        };
        assert_eq!(count(&r), 0, "the minute isn't over yet");
        r.add(&tick(T0 + 61_000, one())).unwrap();
        assert_eq!(count(&r), 1);
        let usage = r
            .store
            .usage(T0, T0 + MINUTE_MS, T0 + MINUTE_MS, 0)
            .unwrap();
        assert_eq!((usage[0].down, usage[0].up), (600, 60));
        assert_eq!(usage[0].packets_down, 60);
        r.flush().unwrap();
        assert_eq!(count(&r), 2);
    }

    #[test]
    fn empty_minutes_write_nothing() {
        let mut r = Recorder::new(Store::open_in_memory().unwrap());
        r.add(&tick(T0 + 1000, Vec::new())).unwrap();
        r.flush().unwrap();
        let n: i64 = r
            .store
            .db
            .query_row("SELECT COUNT(*) FROM apps", [], |x| x.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn reopening_keeps_history() {
        let dir = std::env::temp_dir().join(format!("parcel-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("parcel.db");
        Store::open(&path)
            .unwrap()
            .add(T0, 0, &[app("a", vec![host("x.com", 7, 0)])])
            .unwrap();
        let read = Store::open_read(&path).unwrap();
        let down = read.usage(T0 - HOUR_MS, T0 + HOUR_MS, T0, 0).unwrap()[0].down;
        drop(read);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(down, 7);
    }
}
