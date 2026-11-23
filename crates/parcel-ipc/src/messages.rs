use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to the messages below.
pub const PROTOCOL: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClientMsg {
    Hello { protocol: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerMsg {
    Welcome { protocol: u32, service: String },
    Tick(Tick),
}

/// Traffic over one second, in bytes.
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
pub struct HostRate {
    /// Hostname from DNS or SNI, else the IP address.
    pub host: String,
    pub down: u64,
    pub up: u64,
}
