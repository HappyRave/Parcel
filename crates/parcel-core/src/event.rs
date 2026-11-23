use std::net::{IpAddr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch.
pub type Millis = u64;

#[must_use]
pub fn now_millis() -> Millis {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Proto {
    Tcp,
    Udp,
}

/// Direction as seen from this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    In,
    Out,
}

/// One connection, keyed by its local and remote endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Flow {
    pub proto: Proto,
    pub local: SocketAddr,
    pub remote: SocketAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NameSource {
    Dns,
    Sni,
}

/// Everything a capture source reports. Serializable so a capture can be recorded and replayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// One frame seen on the wire (Npcap). `bytes` is its length on the wire.
    Packet {
        at: Millis,
        flow: Flow,
        dir: Dir,
        bytes: u32,
    },
    /// One send or receive call by a process (ETW). Always names the flow's owner; counts bytes
    /// only when Npcap is absent.
    Transfer {
        at: Millis,
        pid: u32,
        flow: Flow,
        dir: Dir,
        bytes: u32,
    },
    /// A hostname seen for an address, from a DNS reply or a TLS `ClientHello`.
    Hostname {
        at: Millis,
        ip: IpAddr,
        name: String,
        source: NameSource,
    },
    /// A snapshot of the OS's socket table: which process owns each local port.
    Ports { at: Millis, owners: Vec<PortOwner> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortOwner {
    pub proto: Proto,
    pub port: u16,
    pub pid: u32,
}
