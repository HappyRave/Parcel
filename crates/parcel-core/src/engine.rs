use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use parcel_ipc::messages::Tick;

use crate::event::{Dir, Event, Millis, now_millis};

/// Feeds `events` into an engine and calls `on_tick` at every wall-clock second, until every
/// sender is dropped.
pub fn run(events: &Receiver<Event>, npcap: bool, mut on_tick: impl FnMut(Tick)) {
    let mut engine = Engine::new(npcap);
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

/// Folds the capture event stream into per-second totals.
///
/// Byte rule: with Npcap running, only `Packet` events count bytes and ETW just names the owner;
/// without it, ETW's `Transfer` events are the bytes.
pub struct Engine {
    npcap: bool,
    down: u64,
    up: u64,
}

impl Engine {
    #[must_use]
    pub fn new(npcap: bool) -> Self {
        Self {
            npcap,
            down: 0,
            up: 0,
        }
    }

    pub fn apply(&mut self, event: &Event) {
        match *event {
            Event::Packet { dir, bytes, .. } => self.count(dir, bytes),
            Event::Transfer { dir, bytes, .. } if !self.npcap => self.count(dir, bytes),
            Event::Transfer { .. } | Event::Hostname { .. } => {}
        }
    }

    /// Closes the second ending at `at` and starts the next one.
    pub fn tick(&mut self, at: Millis) -> Tick {
        let tick = Tick {
            at,
            down: self.down,
            up: self.up,
        };
        self.down = 0;
        self.up = 0;
        tick
    }

    fn count(&mut self, dir: Dir, bytes: u32) {
        match dir {
            Dir::In => self.down += u64::from(bytes),
            Dir::Out => self.up += u64::from(bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Flow, Proto};

    fn flow() -> Flow {
        Flow {
            proto: Proto::Tcp,
            local: "192.168.1.10:50000".parse().unwrap(),
            remote: "142.250.74.14:443".parse().unwrap(),
        }
    }

    fn packet(dir: Dir, bytes: u32) -> Event {
        Event::Packet {
            at: 0,
            flow: flow(),
            dir,
            bytes,
        }
    }

    fn transfer(dir: Dir, bytes: u32) -> Event {
        Event::Transfer {
            at: 0,
            pid: 42,
            flow: flow(),
            dir,
            bytes,
        }
    }

    #[test]
    fn packets_count_by_direction() {
        let mut engine = Engine::new(true);
        engine.apply(&packet(Dir::In, 1500));
        engine.apply(&packet(Dir::In, 500));
        engine.apply(&packet(Dir::Out, 60));
        assert_eq!(
            engine.tick(1000),
            Tick {
                at: 1000,
                down: 2000,
                up: 60
            }
        );
    }

    #[test]
    fn transfers_are_ignored_while_npcap_counts() {
        let mut engine = Engine::new(true);
        engine.apply(&packet(Dir::In, 1500));
        engine.apply(&transfer(Dir::In, 1460));
        assert_eq!(engine.tick(1000).down, 1500);
    }

    #[test]
    fn transfers_count_without_npcap() {
        let mut engine = Engine::new(false);
        engine.apply(&transfer(Dir::In, 4000));
        engine.apply(&transfer(Dir::Out, 300));
        let tick = engine.tick(1000);
        assert_eq!((tick.down, tick.up), (4000, 300));
    }

    #[test]
    fn tick_starts_a_fresh_second() {
        let mut engine = Engine::new(true);
        engine.apply(&packet(Dir::Out, 100));
        engine.tick(1000);
        assert_eq!(
            engine.tick(2000),
            Tick {
                at: 2000,
                down: 0,
                up: 0
            }
        );
    }

    #[test]
    fn run_ticks_on_second_boundaries_and_stops_when_senders_drop() {
        let (tx, rx) = std::sync::mpsc::channel();
        let feeder = std::thread::spawn(move || {
            tx.send(packet(Dir::In, 100)).unwrap();
            std::thread::sleep(Duration::from_millis(2100));
        });
        let mut ticks = Vec::new();
        run(&rx, true, |t| ticks.push(t));
        feeder.join().unwrap();
        assert!(ticks.len() >= 2, "{ticks:?}");
        assert!(ticks.iter().all(|t| t.at % 1000 == 0));
        assert_eq!(ticks.iter().map(|t| t.down).sum::<u64>(), 100);
    }

    #[test]
    fn events_round_trip_through_json() {
        let event = transfer(Dir::Out, 7);
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }
}
