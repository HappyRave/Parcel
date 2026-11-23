//! Recorded event streams (`parcel-service console --record`) replayed through the engine.
//! Fixtures are hand-written with documentation addresses, never real recordings.

use parcel_core::engine::Engine;
use parcel_core::event::Event;
use parcel_core::process::{App, Processes};
use parcel_ipc::messages::Tick;

struct Names;

impl Processes for Names {
    fn app(&mut self, pid: u32, _image: Option<&str>) -> App {
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
    let mut engine = Engine::new(npcap, Names, Default::default());
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
            std::iter::once(head).chain(a.hosts.iter().map(|h| {
                let service = h.service.as_deref().unwrap_or("-");
                format!(
                    "  {service} {} ↓{} ↑{} pk{}/{}",
                    h.host, h.down, h.up, h.packets_down, h.packets_up
                )
            }))
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
            "  video.example cdn.video.example ↓3028 ↑600 pk2/1",
            // The 900-byte packet came before ETW named its owner; it was held, then claimed.
            "Game ↓1600 ↑0 pk2/0",
            "  - 203.0.113.5 ↓1600 ↑0 pk2/0",
            "Chat ↓0 ↑1350 pk0/1",
            "  - 203.0.113.9 ↓0 ↑1350 pk0/1",
        ]
    );
}

#[test]
fn basic_without_npcap_counts_only_transfers() {
    let tick = replay("basic.jsonl", false);
    assert_eq!((tick.down, tick.up), (5528, 1950));
    // The fixture's packets still count here (real ETW-only captures have none), plus the
    // 900-byte transfer.
    assert_eq!(
        tick.apps.iter().find(|a| a.name == "Game").unwrap().down,
        2500
    );
}
