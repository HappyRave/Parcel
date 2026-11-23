use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch.
pub type Millis = u64;

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
    /// One packet seen on the wire (Npcap). `bytes` is the IP length.
    Packet {
        at: Millis,
        flow: Flow,
        dir: Dir,
        bytes: u32,
    },
    /// One send or receive call by a process (ETW). Counts bytes only when Npcap is absent.
    Transfer {
        at: Millis,
        pid: u32,
        flow: Flow,
        dir: Dir,
        bytes: u32,
    },
    /// A process owns this flow (ETW).
    SocketOwner { at: Millis, pid: u32, flow: Flow },
    /// A hostname seen for an address, from a DNS reply or a TLS `ClientHello`.
    Hostname {
        at: Millis,
        ip: IpAddr,
        name: String,
        source: NameSource,
    },
}
