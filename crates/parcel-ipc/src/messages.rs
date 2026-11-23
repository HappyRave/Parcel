use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to the messages below.
pub const PROTOCOL: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClientMsg {
    Hello {
        protocol: u32,
    },
    /// Answered by a `ServerMsg::Reply` with the same `id`, between ticks.
    Request {
        id: u32,
        request: Request,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerMsg {
    Welcome { protocol: u32, service: String },
    Tick(Tick),
    Reply { id: u32, reply: Reply },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Request {
    /// History totals per app and host between two times (ms since the epoch).
    Usage { from: u64, to: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Reply {
    /// Busiest first; the `AppRate`s hold totals for the whole span.
    Usage {
        apps: Vec<AppRate>,
    },
    Error {
        message: String,
    },
}

/// Traffic over one second, in bytes. (`AppRate` and `HostRate` also carry history totals.)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tick {
    /// End of the second, milliseconds since the Unix epoch.
    pub at: u64,
    pub down: u64,
    pub up: u64,
    /// Apps that moved data this second, busiest first.
    pub apps: Vec<AppRate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRate {
    /// Executable path, the app's identity; empty for traffic with no known owner.
    pub path: String,
    pub name: String,
    pub down: u64,
    pub up: u64,
    /// Zero without Npcap: ETW reports calls, not packets.
    pub packets_down: u64,
    pub packets_up: u64,
    /// Remote hosts this app talked to this second, busiest first.
    pub hosts: Vec<HostRate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRate {
    /// Hostname from DNS or SNI, else the IP address.
    pub host: String,
    /// Brand (`YouTube`), else the registered domain, else (bare IP) the network's owner.
    pub service: Option<String>,
    /// Registered domain, for the favicon.
    pub domain: Option<String>,
    /// ISO 3166-1 alpha-2 country code.
    pub country: Option<String>,
    pub city: Option<String>,
    pub asn: Option<u32>,
    /// AS organization, e.g. `Google LLC`.
    pub network: Option<String>,
    pub down: u64,
    pub up: u64,
    pub packets_down: u64,
    pub packets_up: u64,
}
