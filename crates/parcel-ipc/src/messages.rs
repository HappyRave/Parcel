use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to the messages below.
pub const PROTOCOL: u32 = 1;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tick {
    /// End of the second, milliseconds since the Unix epoch.
    pub at: u64,
    pub down: u64,
    pub up: u64,
}
