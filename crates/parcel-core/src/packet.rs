use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use etherparse::{LaxNetSlice, LaxSlicedPacket, TransportSlice};
use simple_dns::Packet as DnsPacket;
use simple_dns::rdata::RData;
use tls_parser::{
    TlsExtension, TlsMessage, TlsMessageHandshake, parse_tls_extensions, parse_tls_plaintext,
};

use crate::event::{Dir, Event, Flow, Millis, NameSource, Proto};

/// A ClientHello split over more TCP segments than this is given up on.
const SNI_MAX_BYTES: usize = 6 * 1460;
/// Bounds the per-flow SNI bookkeeping; clearing it only costs a re-check of later segments.
const SNI_MAX_FLOWS: usize = 16_384;

/// Turns captured Ethernet frames into events: one `Packet` per TCP / UDP frame, plus `Hostname`s
/// from DNS replies and TLS `ClientHello`s.
pub struct Decoder {
    local: HashSet<IpAddr>,
    sni_pending: HashMap<Flow, Vec<u8>>,
    sni_done: HashSet<Flow>,
}

impl Decoder {
    /// `local` is every address of the captured adapters; it decides each frame's direction.
    #[must_use]
    pub fn new(local: impl IntoIterator<Item = IpAddr>) -> Self {
        Self {
            local: local.into_iter().map(|ip| ip.to_canonical()).collect(),
            sni_pending: HashMap::new(),
            sni_done: HashSet::new(),
        }
    }

    /// Decodes one frame of `wire_len` bytes (`frame` may be truncated by the snap length).
    pub fn decode(
        &mut self,
        at: Millis,
        frame: &[u8],
        wire_len: u32,
        emit: &mut impl FnMut(Event),
    ) {
        let Ok(packet) = LaxSlicedPacket::from_ethernet(frame) else {
            return;
        };
        let (src, dst) = match &packet.net {
            Some(LaxNetSlice::Ipv4(ip)) => (
                IpAddr::V4(ip.header().source_addr()),
                IpAddr::V4(ip.header().destination_addr()),
            ),
            Some(LaxNetSlice::Ipv6(ip)) => (
                IpAddr::V6(ip.header().source_addr()),
                IpAddr::V6(ip.header().destination_addr()),
            ),
            _ => return,
        };
        let (proto, sport, dport, payload) = match &packet.transport {
            Some(TransportSlice::Tcp(t)) => (
                Proto::Tcp,
                t.source_port(),
                t.destination_port(),
                t.payload(),
            ),
            Some(TransportSlice::Udp(u)) => (
                Proto::Udp,
                u.source_port(),
                u.destination_port(),
                u.payload(),
            ),
            _ => return,
        };
        let (src, dst) = (src.to_canonical(), dst.to_canonical());
        let dir = if self.local.contains(&src) {
            Dir::Out
        } else {
            Dir::In
        };
        let flow = match dir {
            Dir::Out => Flow {
                proto,
                local: SocketAddr::new(src, sport),
                remote: SocketAddr::new(dst, dport),
            },
            Dir::In => Flow {
                proto,
                local: SocketAddr::new(dst, dport),
                remote: SocketAddr::new(src, sport),
            },
        };
        emit(Event::Packet {
            at,
            flow,
            dir,
            bytes: wire_len,
        });

        if proto == Proto::Udp && dir == Dir::In && sport == 53 {
            dns_reply(at, payload, emit);
        }
        if proto == Proto::Tcp && dir == Dir::Out && dport == 443 && !payload.is_empty() {
            self.client_hello(at, flow, payload, emit);
        }
    }

    fn client_hello(
        &mut self,
        at: Millis,
        flow: Flow,
        payload: &[u8],
        emit: &mut impl FnMut(Event),
    ) {
        if self.sni_done.contains(&flow) {
            return;
        }
        let buf = self.sni_pending.entry(flow).or_default();
        buf.extend_from_slice(payload);
        match sni(buf) {
            Sni::Found(name) => emit(Event::Hostname {
                at,
                ip: flow.remote.ip(),
                name,
                source: NameSource::Sni,
            }),
            Sni::Incomplete if buf.len() <= SNI_MAX_BYTES => return,
            Sni::Incomplete | Sni::None => {}
        }
        self.sni_pending.remove(&flow);
        if self.sni_done.len() >= SNI_MAX_FLOWS {
            self.sni_done.clear();
        }
        self.sni_done.insert(flow);
    }
}

fn dns_reply(at: Millis, payload: &[u8], emit: &mut impl FnMut(Event)) {
    let Ok(packet) = DnsPacket::parse(payload) else {
        return;
    };
    let Some(question) = packet.questions.first() else {
        return;
    };
    let name = question.qname.to_string();
    for answer in &packet.answers {
        let ip = match &answer.rdata {
            RData::A(a) => IpAddr::V4(Ipv4Addr::from(a.address)),
            RData::AAAA(a) => IpAddr::V6(Ipv6Addr::from(a.address)),
            _ => continue,
        };
        emit(Event::Hostname {
            at,
            ip: ip.to_canonical(),
            name: name.clone(),
            source: NameSource::Dns,
        });
    }
}

enum Sni {
    Found(String),
    /// A TLS record has started but not all of it is here yet.
    Incomplete,
    /// Not a ClientHello, or one without a server name.
    None,
}

fn sni(buf: &[u8]) -> Sni {
    let Ok((_, record)) = parse_tls_plaintext(buf) else {
        if buf.len() >= 5 && buf[0] == 0x16 {
            let need = 5 + usize::from(u16::from_be_bytes([buf[3], buf[4]]));
            if buf.len() < need {
                return Sni::Incomplete;
            }
        }
        return Sni::None;
    };
    for msg in record.msg {
        if let TlsMessage::Handshake(TlsMessageHandshake::ClientHello(hello)) = msg
            && let Some(ext) = hello.ext
            && let Ok((_, exts)) = parse_tls_extensions(ext)
        {
            for e in exts {
                if let TlsExtension::SNI(names) = e
                    && let Some((_, name)) = names.first()
                {
                    return Sni::Found(String::from_utf8_lossy(name).into_owned());
                }
            }
        }
    }
    Sni::None
}

#[cfg(test)]
mod tests {
    use etherparse::PacketBuilder;
    use simple_dns::rdata::{A, AAAA};
    use simple_dns::{CLASS, Name, Question, ResourceRecord, TYPE};

    use super::*;

    const ME: [u8; 4] = [192, 168, 1, 10];
    const THEM: [u8; 4] = [142, 250, 74, 14];
    const RESOLVER: [u8; 4] = [192, 168, 1, 1];

    fn decoder() -> Decoder {
        Decoder::new([IpAddr::from(ME)])
    }

    fn tcp(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PacketBuilder::ethernet2([1; 6], [2; 6])
            .ipv4(src, dst, 64)
            .tcp(sport, dport, 1, 64_000)
            .write(&mut out, payload)
            .unwrap();
        out
    }

    fn udp(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PacketBuilder::ethernet2([1; 6], [2; 6])
            .ipv4(src, dst, 64)
            .udp(sport, dport)
            .write(&mut out, payload)
            .unwrap();
        out
    }

    fn decode_all(d: &mut Decoder, frames: &[Vec<u8>]) -> Vec<Event> {
        let mut events = Vec::new();
        for f in frames {
            d.decode(7, f, u32::try_from(f.len()).unwrap(), &mut |e| {
                events.push(e)
            });
        }
        events
    }

    fn hostnames(events: &[Event]) -> Vec<(IpAddr, String, NameSource)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Hostname {
                    ip, name, source, ..
                } => Some((*ip, name.clone(), *source)),
                _ => None,
            })
            .collect()
    }

    fn client_hello(host: &str) -> Vec<u8> {
        let name = host.as_bytes();
        let name_len = u16::try_from(name.len()).unwrap();
        let mut sni = Vec::new();
        sni.extend_from_slice(&[0x00, 0x00]);
        sni.extend_from_slice(&(name_len + 5).to_be_bytes());
        sni.extend_from_slice(&(name_len + 3).to_be_bytes());
        sni.push(0);
        sni.extend_from_slice(&name_len.to_be_bytes());
        sni.extend_from_slice(name);

        let mut body = vec![0x03, 0x03];
        body.extend_from_slice(&[0xAB; 32]);
        body.push(0);
        body.extend_from_slice(&[0x00, 0x02, 0x13, 0x01]);
        body.extend_from_slice(&[0x01, 0x00]);
        body.extend_from_slice(&u16::try_from(sni.len()).unwrap().to_be_bytes());
        body.extend_from_slice(&sni);

        let mut hs = vec![0x01];
        hs.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes()[1..]);
        hs.extend_from_slice(&body);

        let mut record = vec![0x16, 0x03, 0x01];
        record.extend_from_slice(&u16::try_from(hs.len()).unwrap().to_be_bytes());
        record.extend_from_slice(&hs);
        record
    }

    #[test]
    fn direction_and_flow_follow_the_local_address() {
        let events = decode_all(
            &mut decoder(),
            &[
                tcp(ME, THEM, 50000, 443, b"x"),
                tcp(THEM, ME, 443, 50000, b"yy"),
            ],
        );
        let flow = Flow {
            proto: Proto::Tcp,
            local: "192.168.1.10:50000".parse().unwrap(),
            remote: "142.250.74.14:443".parse().unwrap(),
        };
        let packets: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Packet { flow, dir, .. } => Some((*flow, *dir)),
                _ => None,
            })
            .collect();
        assert_eq!(packets, vec![(flow, Dir::Out), (flow, Dir::In)]);
    }

    #[test]
    fn bytes_are_the_wire_length() {
        let mut d = decoder();
        let frame = tcp(ME, THEM, 50000, 80, &[b'x'; 100]);
        let mut got = None;
        d.decode(0, &frame[..60], 1514, &mut |e| {
            if let Event::Packet { bytes, .. } = e {
                got = Some(bytes);
            }
        });
        assert_eq!(got, Some(1514));
    }

    #[test]
    fn dns_replies_name_their_answers() {
        let mut reply = DnsPacket::new_reply(1);
        let qname = Name::new_unchecked("www.youtube.com");
        reply.questions.push(Question::new(
            qname.clone(),
            TYPE::A.into(),
            CLASS::IN.into(),
            false,
        ));
        reply.answers.push(ResourceRecord::new(
            qname.clone(),
            CLASS::IN,
            60,
            RData::A(A {
                address: u32::from_be_bytes(THEM),
            }),
        ));
        reply.answers.push(ResourceRecord::new(
            qname,
            CLASS::IN,
            60,
            RData::AAAA(AAAA {
                address: 0x2a00_1450_0000_0000_0000_0000_0000_200e,
            }),
        ));
        let bytes = reply.build_bytes_vec().unwrap();
        let events = decode_all(&mut decoder(), &[udp(RESOLVER, ME, 53, 61000, &bytes)]);
        assert_eq!(
            hostnames(&events),
            vec![
                (
                    IpAddr::from(THEM),
                    "www.youtube.com".into(),
                    NameSource::Dns
                ),
                (
                    "2a00:1450::200e".parse().unwrap(),
                    "www.youtube.com".into(),
                    NameSource::Dns
                ),
            ]
        );
    }

    #[test]
    fn dns_queries_we_send_are_not_read_as_replies() {
        let events = decode_all(&mut decoder(), &[udp(ME, RESOLVER, 61000, 53, b"junk")]);
        assert!(hostnames(&events).is_empty());
    }

    #[test]
    fn sni_from_one_segment() {
        let events = decode_all(
            &mut decoder(),
            &[tcp(ME, THEM, 50000, 443, &client_hello("github.com"))],
        );
        assert_eq!(
            hostnames(&events),
            vec![(IpAddr::from(THEM), "github.com".into(), NameSource::Sni)]
        );
    }

    #[test]
    fn sni_split_over_segments() {
        let hello = client_hello("example.org");
        let (a, b) = hello.split_at(30);
        let events = decode_all(
            &mut decoder(),
            &[tcp(ME, THEM, 50000, 443, a), tcp(ME, THEM, 50000, 443, b)],
        );
        assert_eq!(
            hostnames(&events),
            vec![(IpAddr::from(THEM), "example.org".into(), NameSource::Sni)]
        );
    }

    #[test]
    fn sni_is_read_once_per_flow() {
        let hello = client_hello("github.com");
        let events = decode_all(
            &mut decoder(),
            &[
                tcp(ME, THEM, 50000, 443, &hello),
                tcp(ME, THEM, 50000, 443, &hello),
            ],
        );
        assert_eq!(hostnames(&events).len(), 1);
    }

    #[test]
    fn non_tls_payload_gives_no_name() {
        let events = decode_all(
            &mut decoder(),
            &[tcp(ME, THEM, 50000, 443, b"GET / HTTP/1.1\r\n\r\n")],
        );
        assert!(hostnames(&events).is_empty());
    }

    #[test]
    fn garbage_frames_are_skipped() {
        let events = decode_all(&mut decoder(), &[vec![0u8; 10]]);
        assert!(events.is_empty());
    }
}
