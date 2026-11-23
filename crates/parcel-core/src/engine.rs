use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::net::IpAddr;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use parcel_ipc::messages::{AppRate, HostRate, Tick};

use crate::event::{Dir, Event, Flow, Millis, NameSource, Proto, now_millis};
use crate::geo::{Geo, org_name};
use crate::naming::{Named, Naming};
use crate::process::{App, Processes};

/// Port owners, names and app lookups unused for this long are forgotten.
const OWNER_TTL: Millis = 10 * 60_000;
const NAME_TTL: Millis = 60 * 60_000;
const APP_TTL: Millis = 5 * 60_000;
const PRUNE_EVERY: Millis = 60_000;
/// Bytes whose owner isn't known yet wait this many ticks for it (ETW reports in ~1 s batches,
/// often after Npcap's packets) before they count as Unknown app (decided 2026-11-23).
const HOLD_TICKS: u8 = 2;

/// Feeds `events` into an engine and calls `on_tick` at every wall-clock second, until every
/// sender is dropped.
pub fn run<P: Processes>(
    events: &Receiver<Event>,
    npcap: bool,
    processes: P,
    geo: Arc<RwLock<Geo>>,
    mut on_tick: impl FnMut(Tick),
) {
    let mut engine = Engine::new(npcap, processes, geo);
    let mut next = now_millis() / 1000 * 1000 + 1000;
    loop {
        let wait = next.saturating_sub(now_millis());
        match events.recv_timeout(Duration::from_millis(wait)) {
            Ok(event) => engine.apply(&event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = now_millis();
        if now >= next {
            on_tick(engine.tick(next));
            next = now / 1000 * 1000 + 1000;
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Rates {
    down: u64,
    up: u64,
    packets_down: u64,
    packets_up: u64,
}

impl Rates {
    fn add(&mut self, dir: Dir, bytes: u64, packets: u64) {
        match dir {
            Dir::In => {
                self.down += bytes;
                self.packets_down += packets;
            }
            Dir::Out => {
                self.up += bytes;
                self.packets_up += packets;
            }
        }
    }

    fn merge(&mut self, other: Rates) {
        self.down += other.down;
        self.up += other.up;
        self.packets_down += other.packets_down;
        self.packets_up += other.packets_up;
    }
}

#[derive(Default)]
struct Counts {
    total: Rates,
    hosts: HashMap<IpAddr, Rates>,
}

impl Counts {
    fn add(&mut self, ip: IpAddr, dir: Dir, bytes: u64, packets: u64) {
        self.total.add(dir, bytes, packets);
        self.hosts.entry(ip).or_default().add(dir, bytes, packets);
    }

    fn absorb(&mut self, other: Counts) {
        self.total.merge(other.total);
        for (ip, r) in other.hosts {
            self.hosts.entry(ip).or_default().merge(r);
        }
    }
}

/// Folds the capture event stream into per-second totals by app and remote host.
///
/// Byte rule: with Npcap running, only `Packet` events count bytes and ETW just names the owner;
/// without it, ETW's `Transfer` events are the bytes.
pub struct Engine<P> {
    npcap: bool,
    processes: P,
    naming: Naming,
    /// Swapped by the service when a new month's databases arrive.
    geo: Arc<RwLock<Geo>>,
    owners: HashMap<(Proto, u16), (u32, Millis)>,
    names: HashMap<IpAddr, (Named, NameSource, Millis)>,
    apps: HashMap<u32, (App, Millis)>,
    /// Executable paths from process start events, by pid.
    images: HashMap<u32, (String, Millis)>,
    second: HashMap<Option<u32>, Counts>,
    /// Unowned traffic by (protocol, local port), with the ticks it has waited.
    held: HashMap<(Proto, u16), (u8, Counts)>,
    down: u64,
    up: u64,
    pruned: Millis,
}

impl<P: Processes> Engine<P> {
    #[must_use]
    pub fn new(npcap: bool, processes: P, geo: Arc<RwLock<Geo>>) -> Self {
        Self {
            npcap,
            processes,
            naming: Naming::default(),
            geo,
            owners: HashMap::new(),
            names: HashMap::new(),
            apps: HashMap::new(),
            images: HashMap::new(),
            second: HashMap::new(),
            held: HashMap::new(),
            down: 0,
            up: 0,
            pruned: 0,
        }
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            &Event::Packet {
                flow, dir, bytes, ..
            } => {
                let pid = self
                    .owners
                    .get(&(flow.proto, flow.local.port()))
                    .map(|&(pid, _)| pid);
                self.count(pid, flow, dir, bytes, true);
            }
            &Event::Transfer {
                at,
                pid,
                flow,
                dir,
                bytes,
            } => {
                self.learn_owner((flow.proto, flow.local.port()), pid, at);
                if !self.npcap {
                    self.count(Some(pid), flow, dir, bytes, false);
                }
            }
            Event::Ports { at, owners } => {
                for o in owners {
                    self.learn_owner((o.proto, o.port), o.pid, *at);
                }
            }
            Event::Hostname {
                at,
                ip,
                name,
                source,
            } => {
                // SNI is exactly what the app asked for; a later DNS answer doesn't replace it.
                let keep = matches!(
                    (self.names.get(ip), source),
                    (Some((_, NameSource::Sni, _)), NameSource::Dns)
                );
                if !keep {
                    self.names
                        .insert(*ip, (self.naming.name(name), *source, *at));
                }
            }
            Event::ProcessStart { at, pid, image } => {
                // A new process may reuse the pid of one we already named.
                self.apps.remove(pid);
                self.images.insert(*pid, (image.clone(), *at));
            }
        }
    }

    /// The app running as `pid`, named once and cached.
    fn app_of(
        apps: &mut HashMap<u32, (App, Millis)>,
        processes: &mut P,
        images: &HashMap<u32, (String, Millis)>,
        pid: u32,
        at: Millis,
    ) -> App {
        match apps.entry(pid) {
            Entry::Occupied(mut e) => {
                e.get_mut().1 = at;
                e.get().0.clone()
            }
            Entry::Vacant(e) => {
                let image = images.get(&pid).map(|(image, _)| image.as_str());
                e.insert((processes.app(pid, image), at)).0.clone()
            }
        }
    }

    /// Records `pid` as the owner of `key`, naming its app now: a short-lived process may be
    /// gone by the end of the second.
    fn learn_owner(&mut self, key: (Proto, u16), pid: u32, at: Millis) {
        // pid 0 owns sockets in TIME_WAIT after their process exited: it names no one, and
        // must not replace the real owner while that process's last bytes are still held.
        if pid == 0 {
            return;
        }
        self.owners.insert(key, (pid, at));
        Self::app_of(&mut self.apps, &mut self.processes, &self.images, pid, at);
    }

    /// Moves held traffic to its owner once known, or to Unknown app after `HOLD_TICKS`.
    fn release_held(&mut self) {
        for (key, (waited, counts)) in std::mem::take(&mut self.held) {
            let owner = self.owners.get(&key).map(|&(pid, _)| pid);
            if owner.is_some() || waited + 1 >= HOLD_TICKS {
                self.second.entry(owner).or_default().absorb(counts);
            } else {
                self.held.insert(key, (waited + 1, counts));
            }
        }
    }

    /// Closes the second ending at `at` and starts the next one.
    pub fn tick(&mut self, at: Millis) -> Tick {
        self.release_held();
        let geo = self.geo.read().expect("geo lock");
        let mut by_path: HashMap<String, (AppRate, HashMap<String, HostRate>)> = HashMap::new();
        for (pid, counts) in self.second.drain() {
            let app = match pid {
                None => App::unknown(),
                Some(pid) => {
                    Self::app_of(&mut self.apps, &mut self.processes, &self.images, pid, at)
                }
            };
            let (rate, hosts) = by_path.entry(app.path.clone()).or_insert_with(|| {
                (
                    AppRate {
                        path: app.path,
                        name: app.name,
                        ..AppRate::default()
                    },
                    HashMap::new(),
                )
            });
            rate.down += counts.total.down;
            rate.up += counts.total.up;
            rate.packets_down += counts.total.packets_down;
            rate.packets_up += counts.total.packets_up;
            for (ip, r) in counts.hosts {
                let named = self.names.get(&ip).map(|(named, ..)| named);
                let key = named.map_or_else(|| ip.to_string(), |n| n.host.clone());
                let h = hosts.entry(key).or_insert_with_key(|host| {
                    let place = geo.place(ip);
                    // A bare IP is named after whoever runs its network.
                    let service = named.map_or_else(
                        || place.network.as_deref().map(org_name),
                        |n| n.service.clone(),
                    );
                    HostRate {
                        host: host.clone(),
                        service,
                        domain: named.and_then(|n| n.domain.clone()),
                        country: place.country,
                        city: place.city,
                        asn: place.asn,
                        network: place.network,
                        ..HostRate::default()
                    }
                });
                h.down += r.down;
                h.up += r.up;
                h.packets_down += r.packets_down;
                h.packets_up += r.packets_up;
            }
        }
        let mut apps: Vec<AppRate> = by_path
            .into_values()
            .map(|(mut rate, hosts)| {
                rate.hosts = hosts.into_values().collect();
                rate.hosts.sort_by(|a, b| {
                    (b.down + b.up)
                        .cmp(&(a.down + a.up))
                        .then(a.host.cmp(&b.host))
                });
                rate
            })
            .collect();
        apps.sort_by(|a, b| {
            (b.down + b.up)
                .cmp(&(a.down + a.up))
                .then(a.name.cmp(&b.name))
        });
        drop(geo);

        let tick = Tick {
            at,
            down: self.down,
            up: self.up,
            apps,
        };
        self.down = 0;
        self.up = 0;
        if at >= self.pruned + PRUNE_EVERY {
            self.prune(at);
        }
        tick
    }

    fn count(&mut self, pid: Option<u32>, flow: Flow, dir: Dir, bytes: u32, packet: bool) {
        let bytes = u64::from(bytes);
        let packets = u64::from(packet);
        match dir {
            Dir::In => self.down += bytes,
            Dir::Out => self.up += bytes,
        }
        let counts = match pid {
            Some(pid) => self.second.entry(Some(pid)).or_default(),
            None => {
                let key = (flow.proto, flow.local.port());
                &mut self.held.entry(key).or_default().1
            }
        };
        counts.add(flow.remote.ip(), dir, bytes, packets);
    }

    fn prune(&mut self, now: Millis) {
        self.owners
            .retain(|_, (_, seen)| now.saturating_sub(*seen) < OWNER_TTL);
        self.names
            .retain(|_, (.., seen)| now.saturating_sub(*seen) < NAME_TTL);
        self.apps
            .retain(|_, (_, seen)| now.saturating_sub(*seen) < APP_TTL);
        // A process that lived longer is still running when it first talks, so its name
        // comes from the process itself.
        self.images
            .retain(|_, (_, seen)| now.saturating_sub(*seen) < OWNER_TTL);
        self.pruned = now;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::event::PortOwner;

    /// Names every pid `app<pid>` at `C:\app<pid>.exe`; pids 1 and 2 are one app.
    struct Fake;

    impl Processes for Fake {
        fn app(&mut self, pid: u32, _image: Option<&str>) -> App {
            let id = if pid == 2 { 1 } else { pid };
            App {
                path: format!(r"C:\app{id}.exe"),
                name: format!("app{id}"),
            }
        }
    }

    fn flow(port: u16, remote: &str) -> Flow {
        Flow {
            proto: Proto::Tcp,
            local: format!("192.168.1.10:{port}").parse().unwrap(),
            remote: format!("{remote}:443").parse().unwrap(),
        }
    }

    fn packet(port: u16, remote: &str, dir: Dir, bytes: u32) -> Event {
        Event::Packet {
            at: 0,
            flow: flow(port, remote),
            dir,
            bytes,
        }
    }

    fn transfer(pid: u32, port: u16, remote: &str, dir: Dir, bytes: u32) -> Event {
        Event::Transfer {
            at: 0,
            pid,
            flow: flow(port, remote),
            dir,
            bytes,
        }
    }

    fn name(ip: &str, name: &str, source: NameSource) -> Event {
        Event::Hostname {
            at: 0,
            ip: ip.parse().unwrap(),
            name: name.into(),
            source,
        }
    }

    fn engine(npcap: bool, events: &[Event]) -> Engine<Fake> {
        let mut e = Engine::new(npcap, Fake, Arc::default());
        for ev in events {
            e.apply(ev);
        }
        e
    }

    /// Per app: name, down, up, then per host: name, down, up.
    type Summary = Vec<(String, u64, u64, Vec<(String, u64, u64)>)>;

    fn summary(tick: &Tick) -> Summary {
        tick.apps
            .iter()
            .map(|a| {
                let hosts = a
                    .hosts
                    .iter()
                    .map(|h| (h.host.clone(), h.down, h.up))
                    .collect();
                (a.name.clone(), a.down, a.up, hosts)
            })
            .collect()
    }

    #[test]
    fn totals_count_by_direction() {
        let tick = engine(
            true,
            &[
                packet(50000, "1.1.1.1", Dir::In, 1500),
                packet(50000, "1.1.1.1", Dir::In, 500),
                packet(50000, "1.1.1.1", Dir::Out, 60),
            ],
        )
        .tick(1000);
        assert_eq!((tick.at, tick.down, tick.up), (1000, 2000, 60));
    }

    #[test]
    fn transfers_are_ignored_while_npcap_counts() {
        let tick = engine(
            true,
            &[
                packet(50000, "1.1.1.1", Dir::In, 1500),
                transfer(7, 50000, "1.1.1.1", Dir::In, 1460),
            ],
        )
        .tick(1000);
        assert_eq!(tick.down, 1500);
    }

    #[test]
    fn transfers_count_without_npcap_but_not_as_packets() {
        let tick = engine(
            false,
            &[
                transfer(7, 50000, "1.1.1.1", Dir::In, 4000),
                transfer(7, 50000, "1.1.1.1", Dir::Out, 300),
            ],
        )
        .tick(1000);
        assert_eq!((tick.down, tick.up), (4000, 300));
        assert_eq!((tick.apps[0].packets_down, tick.apps[0].packets_up), (0, 0));
    }

    #[test]
    fn packets_with_no_owner_become_unknown_after_the_hold() {
        let mut e = engine(true, &[packet(50000, "1.1.1.1", Dir::In, 100)]);
        e.tick(1000);
        let tick = e.tick(2000);
        assert_eq!(tick.apps[0].name, "Unknown app");
        assert_eq!(tick.apps[0].path, "");
    }

    #[test]
    fn a_transfer_names_the_owner_of_later_packets() {
        let tick = engine(
            true,
            &[
                transfer(7, 50000, "1.1.1.1", Dir::Out, 10),
                packet(50000, "1.1.1.1", Dir::In, 100),
            ],
        )
        .tick(1000);
        assert_eq!(
            summary(&tick),
            vec![("app7".into(), 100, 0, vec![("1.1.1.1".into(), 100, 0)])]
        );
        assert_eq!(tick.apps[0].packets_down, 1);
    }

    #[test]
    fn a_port_snapshot_names_owners() {
        let tick = engine(
            true,
            &[
                Event::Ports {
                    at: 0,
                    owners: vec![PortOwner {
                        proto: Proto::Tcp,
                        port: 50000,
                        pid: 9,
                    }],
                },
                packet(50000, "1.1.1.1", Dir::Out, 40),
            ],
        )
        .tick(1000);
        assert_eq!(tick.apps[0].name, "app9");
    }

    #[test]
    fn processes_of_one_executable_merge_into_one_app() {
        let tick = engine(
            false,
            &[
                transfer(1, 50000, "1.1.1.1", Dir::In, 100),
                transfer(2, 50001, "1.1.1.1", Dir::In, 50),
            ],
        )
        .tick(1000);
        assert_eq!(
            summary(&tick),
            vec![("app1".into(), 150, 0, vec![("1.1.1.1".into(), 150, 0)])]
        );
    }

    #[test]
    fn hosts_take_their_names_and_merge_by_name() {
        let tick = engine(
            false,
            &[
                name("1.1.1.1", "cdn.example.com", NameSource::Dns),
                name("1.1.1.2", "cdn.example.com", NameSource::Dns),
                transfer(7, 50000, "1.1.1.1", Dir::In, 100),
                transfer(7, 50001, "1.1.1.2", Dir::In, 50),
                transfer(7, 50002, "8.8.8.8", Dir::Out, 10),
            ],
        )
        .tick(1000);
        assert_eq!(
            summary(&tick)[0].3,
            vec![
                ("cdn.example.com".into(), 150, 0),
                ("8.8.8.8".into(), 0, 10)
            ]
        );
    }

    #[test]
    fn sni_wins_over_a_later_dns_name() {
        let tick = engine(
            false,
            &[
                name("1.1.1.1", "github.com", NameSource::Sni),
                name("1.1.1.1", "lb-140-82.github.com", NameSource::Dns),
                transfer(7, 50000, "1.1.1.1", Dir::In, 1),
            ],
        )
        .tick(1000);
        assert_eq!(tick.apps[0].hosts[0].host, "github.com");
    }

    #[test]
    fn hosts_carry_service_domain_and_packets() {
        let tick = engine(
            true,
            &[
                name("1.1.1.1", "rr1---sn-abc.GoogleVideo.com.", NameSource::Sni),
                transfer(7, 50000, "1.1.1.1", Dir::Out, 1),
                packet(50000, "1.1.1.1", Dir::In, 1500),
                packet(50000, "1.1.1.1", Dir::In, 1500),
                packet(50000, "1.1.1.1", Dir::Out, 80),
                packet(50000, "9.9.9.9", Dir::Out, 80),
            ],
        )
        .tick(1000);
        let hosts = &tick.apps[0].hosts;
        assert_eq!(
            hosts[0],
            HostRate {
                host: "rr1---sn-abc.googlevideo.com".into(),
                service: Some("YouTube".into()),
                domain: Some("googlevideo.com".into()),
                down: 3000,
                up: 80,
                packets_down: 2,
                packets_up: 1,
                ..HostRate::default()
            }
        );
        assert_eq!(
            (hosts[1].host.as_str(), &hosts[1].service),
            ("9.9.9.9", &None)
        );
    }

    #[test]
    fn apps_are_sorted_busiest_first() {
        let tick = engine(
            false,
            &[
                transfer(3, 50000, "1.1.1.1", Dir::In, 10),
                transfer(4, 50001, "1.1.1.1", Dir::In, 900),
            ],
        )
        .tick(1000);
        let names: Vec<_> = tick.apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["app4", "app3"]);
    }

    #[test]
    fn tick_starts_a_fresh_second() {
        let mut e = engine(
            true,
            &[
                transfer(7, 50000, "1.1.1.1", Dir::Out, 1),
                packet(50000, "1.1.1.1", Dir::Out, 100),
            ],
        );
        e.tick(1000);
        assert_eq!(
            e.tick(2000),
            Tick {
                at: 2000,
                ..Tick::default()
            }
        );
    }

    #[test]
    fn stale_owners_are_forgotten() {
        let mut e = engine(false, &[transfer(7, 50000, "1.1.1.1", Dir::Out, 1)]);
        e.tick(OWNER_TTL + PRUNE_EVERY);
        e.npcap = true;
        e.apply(&packet(50000, "1.1.1.1", Dir::In, 5));
        e.tick(OWNER_TTL + PRUNE_EVERY + 1000);
        assert_eq!(
            e.tick(OWNER_TTL + PRUNE_EVERY + 2000).apps[0].name,
            "Unknown app"
        );
    }

    #[test]
    fn a_late_owner_claims_held_bytes() {
        let mut e = engine(true, &[packet(50000, "1.1.1.1", Dir::In, 100)]);
        assert!(e.tick(1000).apps.is_empty(), "held, not yet unknown");
        e.apply(&transfer(7, 50000, "1.1.1.1", Dir::Out, 1));
        assert_eq!(
            summary(&e.tick(2000)),
            vec![("app7".into(), 100, 0, vec![("1.1.1.1".into(), 100, 0)])]
        );
    }

    #[test]
    fn time_wait_sockets_owned_by_pid_0_dont_replace_the_owner() {
        let ports = |pid| Event::Ports {
            at: 0,
            owners: vec![PortOwner {
                proto: Proto::Tcp,
                port: 50000,
                pid,
            }],
        };
        let mut e = engine(
            true,
            &[ports(7), packet(50000, "1.1.1.1", Dir::In, 100), ports(0)],
        );
        assert_eq!(e.tick(1000).apps[0].name, "app7");
    }

    #[test]
    fn totals_are_not_held() {
        let mut e = engine(true, &[packet(50000, "1.1.1.1", Dir::In, 100)]);
        assert_eq!(e.tick(1000).down, 100);
    }

    /// A pid that exits right after its traffic, like a short curl download.
    struct Exits(HashSet<u32>);

    /// Live pids are curl; a gone pid is named from its start event's image, else a stand-in.
    impl Processes for Exits {
        fn app(&mut self, pid: u32, image: Option<&str>) -> App {
            match (self.0.remove(&pid), image) {
                (true, _) => App {
                    path: r"C:\curl.exe".into(),
                    name: "curl".into(),
                },
                (false, Some(image)) => App {
                    path: image.into(),
                    name: image.rsplit('\\').next().unwrap_or(image).into(),
                },
                (false, None) => App {
                    path: format!("pid:{pid}"),
                    name: format!("Process {pid}"),
                },
            }
        }
    }

    fn start(pid: u32, image: &str) -> Event {
        Event::ProcessStart {
            at: 0,
            pid,
            image: image.into(),
        }
    }

    #[test]
    fn an_exited_process_is_named_from_its_start_event() {
        let mut e = Engine::new(true, Exits(HashSet::new()), Arc::default());
        e.apply(&start(9, r"C:\Tools\wget.exe"));
        e.apply(&transfer(9, 50000, "1.1.1.1", Dir::Out, 1));
        e.apply(&packet(50000, "1.1.1.1", Dir::In, 100));
        let app = &e.tick(1000).apps[0];
        assert_eq!(
            (app.name.as_str(), app.path.as_str()),
            ("wget.exe", r"C:\Tools\wget.exe")
        );
    }

    #[test]
    fn a_reused_pid_is_named_again() {
        let mut e = Engine::new(true, Exits([9].into()), Arc::default());
        e.apply(&transfer(9, 50000, "1.1.1.1", Dir::Out, 1));
        e.apply(&packet(50000, "1.1.1.1", Dir::In, 100));
        assert_eq!(e.tick(1000).apps[0].name, "curl");
        e.apply(&start(9, r"C:\Tools\wget.exe"));
        e.apply(&transfer(9, 50001, "1.1.1.1", Dir::Out, 1));
        e.apply(&packet(50001, "1.1.1.1", Dir::In, 100));
        assert_eq!(e.tick(2000).apps[0].name, "wget.exe");
    }

    #[test]
    fn a_process_is_named_when_first_seen_not_at_the_tick() {
        let mut e = Engine::new(true, Exits([9].into()), Arc::default());
        e.apply(&transfer(9, 50000, "1.1.1.1", Dir::Out, 1));
        e.apply(&packet(50000, "1.1.1.1", Dir::In, 100));
        assert_eq!(e.tick(1000).apps[0].name, "curl");
    }

    #[test]
    fn run_ticks_on_second_boundaries_and_stops_when_senders_drop() {
        let (tx, rx) = std::sync::mpsc::channel();
        let feeder = std::thread::spawn(move || {
            tx.send(packet(50000, "1.1.1.1", Dir::In, 100)).unwrap();
            std::thread::sleep(Duration::from_millis(2100));
        });
        let mut ticks = Vec::new();
        run(&rx, true, Fake, Arc::default(), |t| ticks.push(t));
        feeder.join().unwrap();
        assert!(ticks.len() >= 2, "{ticks:?}");
        assert!(ticks.iter().all(|t| t.at % 1000 == 0));
        assert_eq!(ticks.iter().map(|t| t.down).sum::<u64>(), 100);
    }

    #[test]
    fn events_round_trip_through_json() {
        for event in [
            transfer(7, 50000, "1.1.1.1", Dir::Out, 7),
            Event::Ports {
                at: 1,
                owners: vec![PortOwner {
                    proto: Proto::Udp,
                    port: 53,
                    pid: 4,
                }],
            },
        ] {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
        }
    }
}
