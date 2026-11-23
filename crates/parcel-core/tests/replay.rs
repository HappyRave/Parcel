//! Recorded event streams (`parcel-service console --record`) replayed through the engine.
//! Fixtures are hand-written with documentation addresses, never real recordings.

use parcel_core::engine::Engine;
use parcel_core::event::Event;
use parcel_core::process::{App, Processes};
use parcel_ipc::messages::Tick;

struct Names;

impl Processes for Names {
    fn app(&mut self, pid: u32) -> App {
        let name = match pid {
            100 => "Browser",
            200 => "Game",
            300 => "Chat",
            _ => "Other",
        };
        App {
            path: format!(r"C:\{name}.exe"),
            name: name.to_owned(),
        }
    }
}

fn replay(fixture: &str, npcap: bool) -> Tick {
    let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut engine = Engine::new(npcap, Names);
    for (i, line) in text.lines().enumerate() {
        let event: Event =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{fixture}:{}: {e}", i + 1));
        engine.apply(&event);
    }
    engine.tick(2000)
}

fn rows(tick: &Tick) -> Vec<String> {
    tick.apps
        .iter()
        .flat_map(|a| {
            let head = format!(
                "{} ↓{} ↑{} pk{}/{}",
                a.name, a.down, a.up, a.packets_down, a.packets_up
            );
            std::iter::once(head).chain(
                a.hosts
                    .iter()
                    .map(|h| format!("  {} ↓{} ↑{}", h.host, h.down, h.up)),
            )
        })
        .collect()
}

#[test]
fn basic_with_npcap() {
    let tick = replay("basic.jsonl", true);
    assert_eq!((tick.down, tick.up), (4628, 1950));
    assert_eq!(
        rows(&tick),
        [
            "Browser ↓3028 ↑600 pk2/1",
            "  cdn.video.example ↓3028 ↑600",
            "Chat ↓0 ↑1350 pk0/1",
            "  203.0.113.9 ↓0 ↑1350",
            "Unknown app ↓900 ↑0 pk1/0",
            "  203.0.113.5 ↓900 ↑0",
            "Game ↓700 ↑0 pk1/0",
            "  203.0.113.5 ↓700 ↑0",
        ]
    );
}

#[test]
fn basic_without_npcap_counts_only_transfers() {
    let tick = replay("basic.jsonl", false);
    assert_eq!((tick.down, tick.up), (5528, 1950));
    assert_eq!(
        tick.apps.iter().find(|a| a.name == "Game").unwrap().down,
        1600
    );
}
