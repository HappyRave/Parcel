use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use etherparse::{NetSlice, SlicedPacket, TransportSlice};
use pcap::{Capture, Device};
use simple_dns::Packet as DnsPacket;
use simple_dns::rdata::RData;
use tls_parser::{
    TlsExtension, TlsMessage, TlsMessageHandshake, parse_tls_extensions, parse_tls_plaintext,
};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetExtendedTcpTable, GetExtendedUdpTable, GetIfTable2, MIB_IF_TABLE2,
    MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID,
    TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

const TCP: u8 = 6;
const UDP: u8 = 17;
const SNI_MAX_SEGMENTS: usize = 6;

type Flow = (IpAddr, u16, IpAddr, u16);
type PortMap = HashMap<(u8, u16), u32>;

#[derive(Clone, Copy, Default, Debug)]
struct Counters {
    in_octets: u64,
    out_octets: u64,
    in_pkts: u64,
    out_pkts: u64,
}

#[derive(Default)]
struct Stats {
    by_proto: HashMap<(&'static str, &'static str), (u64, u64)>,
    over_mtu: u64,
    per_remote: HashMap<IpAddr, u64>,
    per_pid: HashMap<u32, u64>,
    unattributed_bytes: u64,
    attributed_bytes: u64,
    dns_names: HashMap<IpAddr, String>,
    sni_names: HashMap<IpAddr, String>,
    sni_pending: HashMap<Flow, Vec<u8>>,
    sni_done: HashSet<Flow>,
    sni_failed: u64,
    quic_bytes: u64,
}

fn guid_string(g: &windows_sys::core::GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

fn hardware_adapters() -> HashMap<String, Counters> {
    let mut out = HashMap::new();
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    let err = unsafe { GetIfTable2(&mut table) };
    assert_eq!(err, 0, "GetIfTable2 failed: {err}");
    unsafe {
        let rows =
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        for row in rows {
            let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if hardware && row.OperStatus == IfOperStatusUp {
                out.insert(
                    guid_string(&row.InterfaceGuid),
                    Counters {
                        in_octets: row.InOctets,
                        out_octets: row.OutOctets,
                        in_pkts: row.InUcastPkts + row.InNUcastPkts,
                        out_pkts: row.OutUcastPkts + row.OutNUcastPkts,
                    },
                );
            }
        }
        FreeMibTable(table as *const _);
    }
    out
}

fn read_table<R: Copy>(fetch: impl Fn(*mut core::ffi::c_void, *mut u32) -> u32) -> Vec<R> {
    let mut size = 0u32;
    fetch(std::ptr::null_mut(), &mut size);
    let mut buf = vec![0u64; (size as usize).div_ceil(8) + 1];
    if fetch(buf.as_mut_ptr().cast(), &mut size) != 0 {
        return Vec::new();
    }
    unsafe {
        let count = *(buf.as_ptr() as *const u32) as usize;
        let rows = (buf.as_ptr() as *const u8).add(std::mem::align_of::<R>().max(4)) as *const R;
        std::slice::from_raw_parts(rows, count).to_vec()
    }
}

fn port(dw: u32) -> u16 {
    u16::from_be(dw as u16)
}

fn port_map() -> PortMap {
    let mut map = PortMap::new();
    let tcp = |af: u16| {
        move |p, s| unsafe { GetExtendedTcpTable(p, s, 0, af as u32, TCP_TABLE_OWNER_PID_ALL, 0) }
    };
    let udp = |af: u16| {
        move |p, s| unsafe { GetExtendedUdpTable(p, s, 0, af as u32, UDP_TABLE_OWNER_PID, 0) }
    };
    for r in read_table::<MIB_TCPROW_OWNER_PID>(tcp(AF_INET)) {
        map.insert((TCP, port(r.dwLocalPort)), r.dwOwningPid);
    }
    for r in read_table::<MIB_TCP6ROW_OWNER_PID>(tcp(AF_INET6)) {
        map.insert((TCP, port(r.dwLocalPort)), r.dwOwningPid);
    }
    for r in read_table::<MIB_UDPROW_OWNER_PID>(udp(AF_INET)) {
        map.insert((UDP, port(r.dwLocalPort)), r.dwOwningPid);
    }
    for r in read_table::<MIB_UDP6ROW_OWNER_PID>(udp(AF_INET6)) {
        map.insert((UDP, port(r.dwLocalPort)), r.dwOwningPid);
    }
    map
}

fn cpu_seconds() -> f64 {
    let mut t = [FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }; 4];
    let [c, e, k, u] = &mut t;
    unsafe { GetProcessTimes(GetCurrentProcess(), c, e, k, u) };
    let secs =
        |f: &FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7;
    secs(k) + secs(u)
}

fn sni(buf: &[u8]) -> Result<Option<String>, ()> {
    let Ok((_, record)) = parse_tls_plaintext(buf) else {
        return if buf.len() >= 5 && buf[0] == 0x16 {
            let need = 5 + u16::from_be_bytes([buf[3], buf[4]]) as usize;
            if buf.len() < need { Ok(None) } else { Err(()) }
        } else {
            Err(())
        };
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
                    return Ok(Some(String::from_utf8_lossy(name).into_owned()));
                }
            }
        }
    }
    Err(())
}

fn on_dns(stats: &mut Stats, payload: &[u8]) {
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
        stats.dns_names.insert(ip, name.clone());
    }
}

fn on_packet(
    stats: &Mutex<Stats>,
    ports: &RwLock<PortMap>,
    local: &HashSet<IpAddr>,
    data: &[u8],
    len: u32,
) {
    let Ok(packet) = SlicedPacket::from_ethernet(data) else {
        return;
    };
    let (src, dst) = match &packet.net {
        Some(NetSlice::Ipv4(ip)) => (
            IpAddr::V4(ip.header().source_addr()),
            IpAddr::V4(ip.header().destination_addr()),
        ),
        Some(NetSlice::Ipv6(ip)) => (
            IpAddr::V6(ip.header().source_addr()),
            IpAddr::V6(ip.header().destination_addr()),
        ),
        _ => return,
    };
    let (proto, sport, dport, payload) = match &packet.transport {
        Some(TransportSlice::Tcp(t)) => (TCP, t.source_port(), t.destination_port(), t.payload()),
        Some(TransportSlice::Udp(u)) => (UDP, u.source_port(), u.destination_port(), u.payload()),
        _ => (0, 0, 0, &[][..]),
    };
    let outbound = local.contains(&src);
    let (remote, local_port) = if outbound { (dst, sport) } else { (src, dport) };
    let size = len as u64;

    let pid = ports.read().unwrap().get(&(proto, local_port)).copied();
    let mut s = stats.lock().unwrap();
    let proto_name = match proto {
        TCP => "tcp",
        UDP => "udp",
        _ => "other",
    };
    let entry = s
        .by_proto
        .entry((proto_name, if outbound { "out" } else { "in" }))
        .or_default();
    entry.0 += 1;
    entry.1 += size;
    if len > 1514 {
        s.over_mtu += 1;
    }
    *s.per_remote.entry(remote).or_default() += size;
    match pid {
        Some(pid) if proto != 0 => {
            s.attributed_bytes += size;
            *s.per_pid.entry(pid).or_default() += size;
        }
        _ => s.unattributed_bytes += size,
    }

    if proto == UDP && sport == 53 {
        on_dns(&mut s, payload);
    }
    if proto == UDP && (sport == 443 || dport == 443) {
        s.quic_bytes += size;
    }
    if proto == TCP && outbound && dport == 443 && !payload.is_empty() {
        let flow = (src, sport, dst, dport);
        if s.sni_done.contains(&flow) {
            return;
        }
        let buf = s.sni_pending.entry(flow).or_default();
        buf.extend_from_slice(payload);
        let result = sni(buf);
        let segments_exhausted = buf.len() > SNI_MAX_SEGMENTS * 1460;
        match result {
            Ok(Some(name)) => {
                s.sni_names.insert(dst, name);
            }
            Ok(None) if !segments_exhausted => return,
            _ if buf.first() == Some(&0x16) => {
                eprintln!(
                    "sni parse failed, {} B: {:02x?}",
                    buf.len(),
                    &buf[..buf.len().min(12)]
                );
                s.sni_failed += 1;
            }
            _ => {}
        }
        s.sni_pending.remove(&flow);
        s.sni_done.insert(flow);
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(60);

    let npcap_dir: Vec<u16> = "C:\\Windows\\System32\\Npcap\0".encode_utf16().collect();
    assert!(
        unsafe { SetDllDirectoryW(npcap_dir.as_ptr()) } != 0,
        "SetDllDirectoryW failed"
    );

    let hardware = hardware_adapters();
    let devices: Vec<Device> = Device::list()
        .expect("pcap device list failed (is Npcap installed?)")
        .into_iter()
        .filter(|d| hardware.keys().any(|g| d.name.ends_with(g.as_str())))
        .collect();
    assert!(
        !devices.is_empty(),
        "no Npcap device matches a hardware adapter"
    );
    let local: HashSet<IpAddr> = devices
        .iter()
        .flat_map(|d| d.addresses.iter().map(|a| a.addr))
        .collect();

    let stats = Arc::new(Mutex::new(Stats::default()));
    let ports = Arc::new(RwLock::new(port_map()));
    let local = Arc::new(local);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let cpu_before = cpu_seconds();
    let before = hardware_adapters();

    let mut threads = Vec::new();
    for device in devices {
        let label = device.desc.clone().unwrap_or(device.name.clone());
        let mut cap = match Capture::from_device(device)
            .and_then(|c| c.snaplen(65535).timeout(200).buffer_size(16 << 20).open())
        {
            Ok(cap) => cap,
            Err(e) => {
                eprintln!("cannot open {label}: {e}");
                continue;
            }
        };
        let (stats, ports, local) = (stats.clone(), ports.clone(), local.clone());
        threads.push(std::thread::spawn(move || {
            while Instant::now() < deadline {
                match cap.next_packet() {
                    Ok(p) => on_packet(&stats, &ports, &local, p.data, p.header.len),
                    Err(pcap::Error::TimeoutExpired) => {}
                    Err(e) => {
                        eprintln!("{label}: {e}");
                        break;
                    }
                }
            }
            (label, cap.stats().ok())
        }));
    }
    assert!(!threads.is_empty(), "no device could be opened");
    println!("capturing {secs} s: browse, stream a video, download something...");
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        *ports.write().unwrap() = port_map();
    }
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let after = hardware_adapters();
    let cpu = cpu_seconds() - cpu_before;

    let mut adapters = Counters::default();
    for (guid, a) in &after {
        if let Some(b) = before.get(guid) {
            adapters.in_octets += a.in_octets - b.in_octets;
            adapters.out_octets += a.out_octets - b.out_octets;
            adapters.in_pkts += a.in_pkts - b.in_pkts;
            adapters.out_pkts += a.out_pkts - b.out_pkts;
        }
    }

    let s = stats.lock().unwrap();
    println!(
        "\n== {secs} s capture, {:.2} s CPU ({:.1}% of one core) ==",
        cpu,
        100.0 * cpu / secs as f64
    );
    for (label, stat) in &results {
        println!("device {label}: {stat:?}");
    }
    println!(
        "adapters: in {:.2} MB / {} pkts, out {:.2} MB / {} pkts",
        mb(adapters.in_octets),
        adapters.in_pkts,
        mb(adapters.out_octets),
        adapters.out_pkts
    );
    let (mut cin, mut cout) = ((0, 0), (0, 0));
    let mut keys: Vec<_> = s.by_proto.keys().collect();
    keys.sort();
    for key in keys {
        let (n, b) = s.by_proto[key];
        println!("npcap {} {}: {n} pkts, {:.2} MB", key.0, key.1, mb(b));
        let t = if key.1 == "in" { &mut cin } else { &mut cout };
        t.0 += n;
        t.1 += b;
    }
    let ratio = |a: u64, b: u64| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    println!(
        "npcap/adapter in: bytes {:.2}, pkts {:.2}; out: bytes {:.2}, pkts {:.2}; frames > 1514 B: {}",
        ratio(cin.1, adapters.in_octets),
        ratio(cin.0, adapters.in_pkts),
        ratio(cout.1, adapters.out_octets),
        ratio(cout.0, adapters.out_pkts),
        s.over_mtu
    );

    let total = s.attributed_bytes + s.unattributed_bytes;
    println!(
        "pid attribution (port table every 500 ms): {:.1}% of bytes, {} pids",
        100.0 * ratio(s.attributed_bytes, total),
        s.per_pid.len()
    );
    let mut pids: Vec<_> = s.per_pid.iter().collect();
    pids.sort_by(|a, b| b.1.cmp(a.1));
    for (pid, bytes) in pids.iter().take(6) {
        println!("  pid {pid:>6}: {:.2} MB", mb(**bytes));
    }

    let named = |m: &HashMap<IpAddr, String>| -> u64 {
        s.per_remote
            .iter()
            .filter(|(ip, _)| m.contains_key(ip))
            .map(|(_, b)| b)
            .sum()
    };
    let remote_total: u64 = s.per_remote.values().sum();
    let either: u64 = s
        .per_remote
        .iter()
        .filter(|(ip, _)| s.dns_names.contains_key(ip) || s.sni_names.contains_key(ip))
        .map(|(_, b)| b)
        .sum();
    println!(
        "names: dns {} ips ({:.1}% of bytes), tls sni {} ips ({:.1}%), either {:.1}%; sni failures {}; quic {:.1}% of bytes",
        s.dns_names.len(),
        100.0 * ratio(named(&s.dns_names), remote_total),
        s.sni_names.len(),
        100.0 * ratio(named(&s.sni_names), remote_total),
        100.0 * ratio(either, remote_total),
        s.sni_failed,
        100.0 * ratio(s.quic_bytes, remote_total)
    );
    let mut remotes: Vec<_> = s.per_remote.iter().collect();
    remotes.sort_by(|a, b| b.1.cmp(a.1));
    for (ip, bytes) in remotes.iter().take(10) {
        let name = s
            .sni_names
            .get(ip)
            .or_else(|| s.dns_names.get(ip))
            .map_or("-", String::as_str);
        println!("  {ip:>39} {:>8.2} MB  {name}", mb(**bytes));
    }
}
