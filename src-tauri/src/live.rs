use std::collections::{HashMap, VecDeque};

use parcel_ipc::messages::{AppRate, Tick};
use serde::Serialize;

/// Seconds in the Live window (graph, app and host totals).
pub const WINDOW: usize = 300;
/// Apps with their own band in the graph; the rest fold into "other" (decided 2026-11-23).
const BANDS: usize = 4;
/// Seconds in a host's sparkline.
const SPARK: usize = 60;
/// Size of the app colour palette; the colours themselves live in the CSS.
const PALETTE: u64 = 10;

/// Everything the Live tab draws, rebuilt every second.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveView {
    pub kpis: Kpis,
    pub graph: Graph,
    /// Busiest first, over the window.
    pub apps: Vec<AppRow>,
    /// The app whose hosts are listed: the picked one, else the busiest.
    pub selected: Option<String>,
    pub hosts: Vec<HostRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kpis {
    /// Bytes in the last second.
    pub down: u64,
    pub up: u64,
    /// Since local midnight.
    pub today_down: u64,
    pub today_up: u64,
    /// `None` without Npcap, which is what counts packets.
    pub packets_per_min: Option<u64>,
    pub apps_now: usize,
    pub hosts_now: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    /// Time of the first slot (ms since the epoch); slots are one second apart.
    pub start: u64,
    /// Highest second in the window, for the ▲ / ▼ scales.
    pub down_max: u64,
    pub up_max: u64,
    /// Per-second totals, for the 2-colour graph.
    pub down: Vec<u64>,
    pub up: Vec<u64>,
    /// Stacked from the axis outwards: the top apps, then "other".
    pub bands: Vec<Band>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Band {
    pub name: String,
    /// Palette index; `None` is grey ("other", Unknown app).
    pub color: Option<u8>,
    pub down: Vec<u64>,
    pub up: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRow {
    pub path: String,
    pub name: String,
    /// File name of the executable, e.g. `chrome.exe`; empty for System and stand-ins.
    pub exe: String,
    pub color: Option<u8>,
    pub down: u64,
    pub up: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRow {
    pub host: String,
    /// Palette index for the letter tile that stands in for the favicon.
    pub color: Option<u8>,
    pub service: Option<String>,
    pub domain: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub asn: Option<u32>,
    pub network: Option<String>,
    pub packets: u64,
    pub down: u64,
    pub up: u64,
    /// Bytes per second, both ways, over the last `SPARK` seconds.
    pub spark: Vec<u64>,
}

/// A stable colour for a key (an app's path, a host's service): hashed into the palette with
/// FNV-1a, so it never changes across sessions.
#[must_use]
pub fn color_of(path: &str) -> Option<u8> {
    if path.is_empty() {
        return None;
    }
    let hash = path.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    u8::try_from(hash % PALETTE).ok()
}

/// Local midnight before `now_ms`, for a UTC offset in seconds.
#[must_use]
pub fn local_midnight(now_ms: u64, offset: i64) -> u64 {
    let local = i64::try_from(now_ms / 1000).unwrap_or(i64::MAX) + offset;
    let midnight = local - local.rem_euclid(86_400) - offset;
    u64::try_from(midnight).unwrap_or(0) * 1000
}

fn exe_of(path: &str) -> String {
    if !path.contains('\\') {
        return String::new();
    }
    path.rsplit('\\').next().unwrap_or_default().to_owned()
}

/// The last `WINDOW` seconds of ticks and what the user picked.
#[derive(Default)]
pub struct Live {
    ticks: VecDeque<Tick>,
    selected: Option<String>,
    npcap: bool,
    /// Today's totals from history, complete up to `today_as_of`.
    today: (u64, u64),
    today_as_of: u64,
}

impl Live {
    /// Starts over for a new connection.
    pub fn reset(&mut self, npcap: bool) {
        self.ticks.clear();
        self.npcap = npcap;
        self.today = (0, 0);
        self.today_as_of = 0;
    }

    /// Adds a tick; one already seen (same or older `at`) is skipped.
    pub fn push(&mut self, tick: Tick) {
        if self.ticks.back().is_some_and(|t| t.at >= tick.at) {
            return;
        }
        if self.ticks.len() == WINDOW {
            self.ticks.pop_front();
        }
        self.ticks.push_back(tick);
    }

    pub fn select(&mut self, path: Option<String>) {
        self.selected = path;
    }

    /// History's totals since midnight, complete up to `as_of`; later ticks are added live.
    pub fn set_today(&mut self, down: u64, up: u64, as_of: u64) {
        self.today = (down, up);
        self.today_as_of = as_of;
    }

    #[must_use]
    pub fn view(&self) -> LiveView {
        let Some(last) = self.ticks.back() else {
            return LiveView::default();
        };
        let start = last.at.saturating_sub((WINDOW as u64 - 1) * 1000);
        let slot = |at: u64| usize::try_from((at - start) / 1000).ok();

        // Window totals per app, keeping each app's latest name.
        let mut totals: HashMap<&str, (&str, u64, u64)> = HashMap::new();
        for t in &self.ticks {
            for a in &t.apps {
                let e = totals.entry(&a.path).or_insert((&a.name, 0, 0));
                e.0 = &a.name;
                e.1 += a.down;
                e.2 += a.up;
            }
        }
        let mut apps: Vec<AppRow> = totals
            .into_iter()
            .map(|(path, (name, down, up))| AppRow {
                path: path.to_owned(),
                name: name.to_owned(),
                exe: exe_of(path),
                color: color_of(path),
                down,
                up,
            })
            .collect();
        // Listed by download, the number each row shows, like the hosts' "sorted by ↓".
        apps.sort_by(|a, b| {
            b.down
                .cmp(&a.down)
                .then(b.up.cmp(&a.up))
                .then(a.name.cmp(&b.name))
        });

        let graph = self.graph(start, &apps, slot);
        let selected = self
            .selected
            .as_ref()
            .filter(|p| apps.iter().any(|a| &a.path == *p))
            .or_else(|| apps.first().map(|a| &a.path))
            .cloned();
        let hosts = selected
            .as_deref()
            .map(|path| self.hosts(path, last.at))
            .unwrap_or_default();
        LiveView {
            kpis: self.kpis(last),
            graph,
            apps,
            selected,
            hosts,
        }
    }

    fn kpis(&self, last: &Tick) -> Kpis {
        let (mut today_down, mut today_up) = self.today;
        for t in self.ticks.iter().filter(|t| t.at > self.today_as_of) {
            today_down += t.down;
            today_up += t.up;
        }
        let minute = self.ticks.iter().rev().take(60);
        let packets = minute
            .flat_map(|t| &t.apps)
            .map(|a| a.packets_down + a.packets_up)
            .sum();
        let mut hosts: Vec<&str> = last
            .apps
            .iter()
            .flat_map(|a| a.hosts.iter().map(|h| h.host.as_str()))
            .collect();
        hosts.sort_unstable();
        hosts.dedup();
        Kpis {
            down: last.down,
            up: last.up,
            today_down,
            today_up,
            packets_per_min: self.npcap.then_some(packets),
            apps_now: last.apps.len(),
            hosts_now: hosts.len(),
        }
    }

    fn graph(&self, start: u64, apps: &[AppRow], slot: impl Fn(u64) -> Option<usize>) -> Graph {
        // Bands go to the busiest apps both ways, so a big uploader gets one too.
        let mut top: Vec<&AppRow> = apps.iter().filter(|a| !a.path.is_empty()).collect();
        top.sort_by(|a, b| {
            (b.down + b.up)
                .cmp(&(a.down + a.up))
                .then(a.name.cmp(&b.name))
        });
        top.truncate(BANDS);
        let mut bands: Vec<Band> = top
            .iter()
            .map(|a| Band {
                name: a.name.clone(),
                color: a.color,
                down: vec![0; WINDOW],
                up: vec![0; WINDOW],
            })
            .collect();
        let mut other = Band {
            name: "other".to_owned(),
            color: None,
            down: vec![0; WINDOW],
            up: vec![0; WINDOW],
        };
        let (mut down, mut up) = (vec![0; WINDOW], vec![0; WINDOW]);
        for t in &self.ticks {
            let Some(i) = slot(t.at).filter(|&i| i < WINDOW) else {
                continue;
            };
            down[i] = t.down;
            up[i] = t.up;
            for a in &t.apps {
                let band = match top.iter().position(|b| b.path == a.path) {
                    Some(k) => &mut bands[k],
                    None => &mut other,
                };
                band.down[i] += a.down;
                band.up[i] += a.up;
            }
        }
        if other.down.iter().chain(&other.up).any(|&v| v > 0) {
            bands.push(other);
        }
        let stacked = |pick: fn(&Band) -> &Vec<u64>, i: usize| -> u64 {
            bands.iter().map(|b| pick(b)[i]).sum()
        };
        let down_max = (0..WINDOW)
            .map(|i| down[i].max(stacked(|b| &b.down, i)))
            .max()
            .unwrap_or(0);
        let up_max = (0..WINDOW)
            .map(|i| up[i].max(stacked(|b| &b.up, i)))
            .max()
            .unwrap_or(0);
        Graph {
            start,
            down_max,
            up_max,
            down,
            up,
            bands,
        }
    }

    fn hosts(&self, path: &str, last_at: u64) -> Vec<HostRow> {
        let spark_start = last_at.saturating_sub((SPARK as u64 - 1) * 1000);
        let mut rows: HashMap<&str, HostRow> = HashMap::new();
        for t in &self.ticks {
            let Some(app) = t.apps.iter().find(|a: &&AppRate| a.path == path) else {
                continue;
            };
            for h in &app.hosts {
                let row = rows.entry(&h.host).or_insert_with(|| HostRow {
                    host: h.host.clone(),
                    spark: vec![0; SPARK],
                    ..HostRow::default()
                });
                // Newer ticks carry newer names and places.
                row.service.clone_from(&h.service);
                row.domain.clone_from(&h.domain);
                row.country.clone_from(&h.country);
                row.city.clone_from(&h.city);
                row.asn = h.asn;
                row.network.clone_from(&h.network);
                // An unnamed address gets the grey "?" tile.
                row.color = h.service.as_deref().and_then(color_of);
                row.packets += h.packets_down + h.packets_up;
                row.down += h.down;
                row.up += h.up;
                if t.at >= spark_start {
                    let i = usize::try_from((t.at - spark_start) / 1000).unwrap_or(SPARK);
                    if let Some(v) = row.spark.get_mut(i) {
                        *v += h.down + h.up;
                    }
                }
            }
        }
        let mut rows: Vec<HostRow> = rows.into_values().collect();
        rows.sort_by(|a, b| {
            b.down
                .cmp(&a.down)
                .then(b.up.cmp(&a.up))
                .then(a.host.cmp(&b.host))
        });
        rows
    }
}

#[cfg(test)]
mod tests {
    use parcel_ipc::messages::HostRate;

    use super::*;

    const T: u64 = 1_795_440_000_000;

    fn host(name: &str, down: u64) -> HostRate {
        HostRate {
            host: name.into(),
            service: Some(name.to_uppercase()),
            down,
            up: down / 10,
            packets_down: 1,
            packets_up: 1,
            ..HostRate::default()
        }
    }

    fn app(path: &str, hosts: Vec<HostRate>) -> AppRate {
        AppRate {
            path: path.into(),
            name: path.rsplit('\\').next().unwrap().into(),
            down: hosts.iter().map(|h| h.down).sum(),
            up: hosts.iter().map(|h| h.up).sum(),
            packets_down: hosts.len() as u64,
            packets_up: hosts.len() as u64,
            hosts,
        }
    }

    fn tick(at: u64, apps: Vec<AppRate>) -> Tick {
        Tick {
            at,
            down: apps.iter().map(|a| a.down).sum(),
            up: apps.iter().map(|a| a.up).sum(),
            apps,
        }
    }

    fn live(ticks: Vec<Tick>) -> Live {
        let mut l = Live::default();
        l.reset(true);
        for t in ticks {
            l.push(t);
        }
        l
    }

    #[test]
    fn colours_are_stable_and_in_the_palette() {
        let c = color_of(r"C:\Program Files\Google\Chrome\Application\chrome.exe");
        assert_eq!(
            c,
            color_of(r"C:\Program Files\Google\Chrome\Application\chrome.exe")
        );
        assert!(c.is_some_and(|c| u64::from(c) < PALETTE));
        assert_eq!(color_of(""), None);
    }

    #[test]
    fn midnight_is_local() {
        // 2026-11-23 13:20 UTC; at UTC+1 local midnight is 23:00 UTC the day before.
        let utc_midnight = T - (T / 1000 % 86_400) * 1000;
        assert_eq!(local_midnight(T, 0), utc_midnight);
        assert_eq!(local_midnight(T, 3600), utc_midnight - 3_600_000);
        // At 23:30 UTC, UTC+1 is already 00:30 the next day.
        let late = utc_midnight + 23 * 3_600_000 + 1_800_000;
        assert_eq!(local_midnight(late, 3600), late - 1_800_000);
    }

    #[test]
    fn empty_until_the_first_tick() {
        assert_eq!(Live::default().view(), LiveView::default());
    }

    #[test]
    fn old_or_repeated_ticks_are_skipped() {
        let mut l = live(vec![tick(T, vec![]), tick(T + 1000, vec![])]);
        l.push(tick(T + 1000, vec![app(r"C:\a.exe", vec![host("x", 5)])]));
        l.push(tick(T, vec![]));
        assert_eq!(l.ticks.len(), 2);
        assert!(l.view().apps.is_empty());
    }

    #[test]
    fn the_window_keeps_five_minutes() {
        let l = live((0..400).map(|s| tick(T + s * 1000, vec![])).collect());
        assert_eq!(l.ticks.len(), WINDOW);
        let g = l.view().graph;
        assert_eq!(g.start, T + 100_000);
        assert_eq!(g.down.len(), WINDOW);
    }

    #[test]
    fn apps_total_the_window_busiest_first() {
        let l = live(vec![
            tick(
                T,
                vec![
                    app(r"C:\a.exe", vec![host("x", 100)]),
                    app(r"C:\b.exe", vec![host("y", 50)]),
                ],
            ),
            tick(T + 1000, vec![app(r"C:\b.exe", vec![host("y", 200)])]),
        ]);
        let v = l.view();
        let rows: Vec<_> = v.apps.iter().map(|a| (a.exe.as_str(), a.down)).collect();
        assert_eq!(rows, [("b.exe", 250), ("a.exe", 100)]);
        assert_eq!(v.selected.as_deref(), Some(r"C:\b.exe"));
    }

    #[test]
    fn the_list_follows_download_while_bands_follow_both_ways() {
        let uploader = AppRate {
            path: r"C:\backup.exe".into(),
            name: "backup.exe".into(),
            down: 10,
            up: 10_000,
            hosts: vec![host("s", 10)],
            ..AppRate::default()
        };
        let mut apps: Vec<AppRate> = (0..4)
            .map(|k| app(&format!(r"C:\app{k}.exe"), vec![host("h", 1000 + k)]))
            .collect();
        apps.push(uploader);
        let v = live(vec![tick(T, apps)]).view();
        assert_eq!(v.apps.last().map(|a| a.name.as_str()), Some("backup.exe"));
        assert_eq!(v.graph.bands[0].name, "backup.exe");
    }

    #[test]
    fn unnamed_hosts_get_no_colour() {
        let mut bare = host("10.0.0.1", 5);
        bare.service = None;
        let v = live(vec![tick(
            T,
            vec![app(r"C:\a.exe", vec![bare, host("x", 1)])],
        )])
        .view();
        let colours: Vec<_> = v
            .hosts
            .iter()
            .map(|h| (h.host.as_str(), h.color.is_some()))
            .collect();
        assert_eq!(colours, [("10.0.0.1", false), ("x", true)]);
    }

    #[test]
    fn graph_gives_four_apps_bands_and_folds_the_rest() {
        let apps = (0..6)
            .map(|k| app(&format!(r"C:\app{k}.exe"), vec![host("h", 100 * (6 - k))]))
            .collect();
        let g = live(vec![tick(T, apps)]).view().graph;
        let names: Vec<_> = g.bands.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            ["app0.exe", "app1.exe", "app2.exe", "app3.exe", "other"]
        );
        let last = WINDOW - 1;
        assert_eq!(g.bands[4].down[last], 200 + 100);
        assert_eq!(g.bands[4].color, None);
        assert_eq!(g.down_max, 2100);
    }

    #[test]
    fn unknown_app_is_never_a_named_band() {
        let g = live(vec![tick(
            T,
            vec![
                app("", vec![host("h", 999)]),
                app(r"C:\a.exe", vec![host("h", 1)]),
            ],
        )])
        .view()
        .graph;
        let names: Vec<_> = g.bands.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["a.exe", "other"]);
    }

    #[test]
    fn gaps_stay_zero_in_the_series() {
        let g = live(vec![
            tick(T, vec![app(r"C:\a.exe", vec![host("x", 10)])]),
            tick(T + 3000, vec![app(r"C:\a.exe", vec![host("x", 30)])]),
        ])
        .view()
        .graph;
        let tail: Vec<u64> = g.bands[0].down[WINDOW - 4..].to_vec();
        assert_eq!(tail, [10, 0, 0, 30]);
    }

    #[test]
    fn picked_app_lists_its_hosts_with_sparklines() {
        let mut l = live(vec![
            tick(
                T,
                vec![
                    app(r"C:\a.exe", vec![host("x", 10)]),
                    app(r"C:\b.exe", vec![host("y", 900)]),
                ],
            ),
            tick(
                T + 1000,
                vec![app(r"C:\a.exe", vec![host("x", 20), host("z", 50)])],
            ),
        ]);
        l.select(Some(r"C:\a.exe".into()));
        let v = l.view();
        assert_eq!(v.selected.as_deref(), Some(r"C:\a.exe"));
        let hosts: Vec<_> = v
            .hosts
            .iter()
            .map(|h| (h.host.as_str(), h.down, h.packets))
            .collect();
        assert_eq!(hosts, [("z", 50, 2), ("x", 30, 4)]);
        let x = &v.hosts[1];
        assert_eq!(x.spark[SPARK - 2..], [11, 22]);
        assert_eq!(x.service.as_deref(), Some("X"));
    }

    #[test]
    fn a_gone_pick_falls_back_to_the_busiest() {
        let mut l = live(vec![tick(T, vec![app(r"C:\a.exe", vec![host("x", 10)])])]);
        l.select(Some(r"C:\gone.exe".into()));
        assert_eq!(l.view().selected.as_deref(), Some(r"C:\a.exe"));
    }

    #[test]
    fn kpis_add_live_seconds_to_history_and_count_now() {
        let mut l = live(vec![
            tick(T, vec![app(r"C:\a.exe", vec![host("x", 100)])]),
            tick(
                T + 1000,
                vec![
                    app(r"C:\a.exe", vec![host("x", 10), host("y", 5)]),
                    app(r"C:\b.exe", vec![host("x", 1)]),
                ],
            ),
        ]);
        l.set_today(5000, 500, T);
        let k = l.view().kpis;
        assert_eq!((k.down, k.up), (16, 1));
        assert_eq!((k.today_down, k.today_up), (5016, 501));
        assert_eq!((k.apps_now, k.hosts_now), (2, 2));
        assert_eq!(k.packets_per_min, Some(2 + 6));
    }

    #[test]
    fn no_packet_count_without_npcap() {
        let mut l = live(vec![tick(T, vec![])]);
        l.reset(false);
        l.push(tick(T, vec![]));
        assert_eq!(l.view().kpis.packets_per_min, None);
    }

    #[test]
    fn exe_names_only_for_real_paths() {
        assert_eq!(exe_of(r"C:\Windows\System32\curl.exe"), "curl.exe");
        assert_eq!(exe_of("System"), "");
        assert_eq!(exe_of("pid:42"), "");
    }
}
