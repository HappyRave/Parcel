use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrisetw::EventRecord;
use ferrisetw::parser::Parser;
use ferrisetw::provider::{Provider, kernel_providers};
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::{KernelTrace, UserTrace};
use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;

const DNS_CLIENT: &str = "1c95126e-7eea-49a9-a3fe-a378b03ddb4d";
const KERNEL_NETWORK: &str = "7dd42a49-5329-4832-8dfd-43d979153a88";
const MTU: u32 = 1500;
const SAMPLES: usize = 6;

#[derive(Default)]
struct Bucket {
    events: u64,
    bytes: u64,
    over_mtu: u64,
}

#[derive(Default)]
struct Stats {
    buckets: HashMap<(&'static str, &'static str), Bucket>,
    per_pid: HashMap<u32, u64>,
    per_remote: HashMap<IpAddr, u64>,
    pid_zero_bytes: u64,
    loopback_bytes: u64,
    sizes: Vec<u32>,
    samples: Vec<String>,
    parse_errors: u64,
    dns_events: u64,
    dns_names: HashMap<IpAddr, String>,
    dns_hist: HashMap<u16, u64>,
    kernel_hist: HashMap<(String, u8), (u64, u64)>,
    modern_hist: HashMap<u16, (u64, u64)>,
}

#[derive(Clone, Copy, Default)]
struct Counters {
    in_octets: u64,
    out_octets: u64,
    in_pkts: u64,
    out_pkts: u64,
}

fn adapter_counters() -> Counters {
    let mut total = Counters::default();
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    let err = unsafe { GetIfTable2(&mut table) };
    assert_eq!(err, 0, "GetIfTable2 failed: {err}");
    unsafe {
        let rows =
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        for row in rows {
            let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if hardware && row.OperStatus == IfOperStatusUp {
                total.in_octets += row.InOctets;
                total.out_octets += row.OutOctets;
                total.in_pkts += row.InUcastPkts + row.InNUcastPkts;
                total.out_pkts += row.OutUcastPkts + row.OutNUcastPkts;
            }
        }
        FreeMibTable(table as *const _);
    }
    total
}

fn on_net_event(stats: &Mutex<Stats>, record: &EventRecord, locator: &SchemaLocator) {
    let Ok(schema) = locator.event_schema(record) else {
        stats.lock().unwrap().parse_errors += 1;
        return;
    };
    let parser = Parser::create(record, &schema);
    let size: Option<u32> = parser.try_parse("size").ok();
    {
        let mut s = stats.lock().unwrap();
        let entry = s
            .kernel_hist
            .entry((schema.provider_name(), record.opcode()))
            .or_default();
        entry.0 += 1;
        entry.1 += size.unwrap_or(0) as u64;
    }
    let direction = match record.opcode() {
        10 | 26 => "send",
        11 | 27 => "recv",
        _ => return,
    };
    let proto = if schema.provider_name().contains("Udp") {
        "udp"
    } else {
        "tcp"
    };
    let pid: Option<u32> = parser.try_parse("PID").ok();
    let saddr: Option<IpAddr> = parser.try_parse("saddr").ok();
    let daddr: Option<IpAddr> = parser.try_parse("daddr").ok();
    let (Some(size), Some(pid), Some(saddr), Some(daddr)) = (size, pid, saddr, daddr) else {
        stats.lock().unwrap().parse_errors += 1;
        return;
    };

    let mut s = stats.lock().unwrap();
    if daddr.is_loopback() {
        s.loopback_bytes += size as u64;
        return;
    }
    let bucket = s.buckets.entry((proto, direction)).or_default();
    bucket.events += 1;
    bucket.bytes += size as u64;
    if size > MTU {
        bucket.over_mtu += 1;
    }
    *s.per_pid.entry(pid).or_default() += size as u64;
    if pid == 0 {
        s.pid_zero_bytes += size as u64;
    }
    *s.per_remote.entry(daddr.to_canonical()).or_default() += size as u64;
    s.sizes.push(size);
    if s.samples.len() < SAMPLES && direction == "recv" {
        s.samples.push(format!(
            "{proto} recv pid={pid} size={size} saddr={saddr} daddr={daddr} record.pid={}",
            record.process_id()
        ));
    }
}

fn on_modern_event(stats: &Mutex<Stats>, record: &EventRecord, locator: &SchemaLocator) {
    let size = locator.event_schema(record).ok().and_then(|schema| {
        Parser::create(record, &schema)
            .try_parse::<u32>("size")
            .ok()
    });
    let mut s = stats.lock().unwrap();
    let entry = s.modern_hist.entry(record.event_id()).or_default();
    entry.0 += 1;
    entry.1 += size.unwrap_or(0) as u64;
}

fn on_dns_event(stats: &Mutex<Stats>, record: &EventRecord, locator: &SchemaLocator) {
    let Ok(schema) = locator.event_schema(record) else {
        return;
    };
    let parser = Parser::create(record, &schema);
    let mut s = stats.lock().unwrap();
    s.dns_events += 1;
    *s.dns_hist.entry(record.event_id()).or_default() += 1;
    let (Ok(name), Ok(results)) = (
        parser.try_parse::<String>("QueryName"),
        parser.try_parse::<String>("QueryResults"),
    ) else {
        return;
    };
    for ip in results
        .split(';')
        .filter_map(|t| t.trim().parse::<IpAddr>().ok())
    {
        s.dns_names.insert(ip.to_canonical(), name.clone());
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

fn report(stats: &Stats, before: Counters, after: Counters, secs: u64) {
    let in_octets = after.in_octets - before.in_octets;
    let out_octets = after.out_octets - before.out_octets;
    let in_pkts = after.in_pkts - before.in_pkts;
    let out_pkts = after.out_pkts - before.out_pkts;

    println!("\n== {secs} s capture ==");
    println!(
        "adapters (hardware, up): in {:.2} MB / {in_pkts} pkts, out {:.2} MB / {out_pkts} pkts",
        mb(in_octets),
        mb(out_octets)
    );

    let mut etw_in = (0, 0);
    let mut etw_out = (0, 0);
    let mut keys: Vec<_> = stats.buckets.keys().collect();
    keys.sort();
    for key in keys {
        let b = &stats.buckets[key];
        println!(
            "etw {} {}: {} events, {:.2} MB, {} events > {MTU} B",
            key.0,
            key.1,
            b.events,
            mb(b.bytes),
            b.over_mtu
        );
        let total = if key.1 == "recv" {
            &mut etw_in
        } else {
            &mut etw_out
        };
        total.0 += b.events;
        total.1 += b.bytes;
    }
    let ratio = |a: u64, b: u64| {
        if b == 0 { 0.0 } else { a as f64 / b as f64 }
    };
    println!(
        "etw/adapter in: bytes {:.2}, events/pkts {:.2}",
        ratio(etw_in.1, in_octets),
        ratio(etw_in.0, in_pkts)
    );
    println!(
        "etw/adapter out: bytes {:.2}, events/pkts {:.2}",
        ratio(etw_out.1, out_octets),
        ratio(etw_out.0, out_pkts)
    );

    let mut sizes = stats.sizes.clone();
    sizes.sort_unstable();
    if !sizes.is_empty() {
        let p = |q: f64| sizes[((sizes.len() - 1) as f64 * q) as usize];
        println!(
            "event size: p50 {} p90 {} p99 {} max {}",
            p(0.5),
            p(0.9),
            p(0.99),
            sizes[sizes.len() - 1]
        );
    }

    let total: u64 = stats.per_pid.values().sum();
    println!(
        "pid 0 share: {:.1}%, loopback skipped: {:.2} MB, parse errors: {}",
        100.0 * ratio(stats.pid_zero_bytes, total),
        mb(stats.loopback_bytes),
        stats.parse_errors
    );
    let mut pids: Vec<_> = stats.per_pid.iter().collect();
    pids.sort_by(|a, b| b.1.cmp(a.1));
    for (pid, bytes) in pids.iter().take(8) {
        println!("  pid {pid:>6}: {:.2} MB", mb(**bytes));
    }

    let named: u64 = stats
        .per_remote
        .iter()
        .filter(|(ip, _)| stats.dns_names.contains_key(ip))
        .map(|(_, b)| b)
        .sum();
    let remote_total: u64 = stats.per_remote.values().sum();
    println!(
        "dns: {} events, {} ips named; remotes named: {}/{} ({:.1}% of bytes)",
        stats.dns_events,
        stats.dns_names.len(),
        stats
            .per_remote
            .keys()
            .filter(|ip| stats.dns_names.contains_key(ip))
            .count(),
        stats.per_remote.len(),
        100.0 * ratio(named, remote_total)
    );
    let mut remotes: Vec<_> = stats.per_remote.iter().collect();
    remotes.sort_by(|a, b| b.1.cmp(a.1));
    for (ip, bytes) in remotes.iter().take(10) {
        let name = stats.dns_names.get(ip).map_or("-", String::as_str);
        println!("  {ip:>39} {:>8.2} MB  {name}", mb(**bytes));
    }

    println!("kernel logger, all events (provider, opcode): events, MB of `size`");
    let mut hist: Vec<_> = stats.kernel_hist.iter().collect();
    hist.sort();
    for ((provider, opcode), (n, bytes)) in hist {
        println!("  {provider:>8} op {opcode:>3}: {n:>7} {:>8.2}", mb(*bytes));
    }
    println!("Microsoft-Windows-Kernel-Network (event id): events, MB of `size`");
    let mut hist: Vec<_> = stats.modern_hist.iter().collect();
    hist.sort();
    for (id, (n, bytes)) in hist {
        println!("  id {id:>3}: {n:>7} {:>8.2}", mb(*bytes));
    }
    let mut hist: Vec<_> = stats.dns_hist.iter().collect();
    hist.sort();
    println!("dns events by id: {hist:?}");

    println!("recv samples (which field is the remote?):");
    for line in &stats.samples {
        println!("  {line}");
    }
}

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(30);
    let stats = Arc::new(Mutex::new(Stats::default()));

    let net_stats = stats.clone();
    let net = Provider::kernel(&kernel_providers::TCP_IP_PROVIDER)
        .add_callback(move |r: &EventRecord, l: &SchemaLocator| on_net_event(&net_stats, r, l))
        .build();
    let dns_stats = stats.clone();
    let dns = Provider::by_guid(DNS_CLIENT)
        .add_callback(move |r: &EventRecord, l: &SchemaLocator| on_dns_event(&dns_stats, r, l))
        .build();
    let modern_stats = stats.clone();
    let modern = Provider::by_guid(KERNEL_NETWORK)
        .add_callback(move |r: &EventRecord, l: &SchemaLocator| {
            on_modern_event(&modern_stats, r, l)
        })
        .build();

    for stale in ["ParcelEtwSpikeDns", "ParcelEtwSpike"] {
        if ferrisetw::trace::stop_trace_by_name(stale).is_ok() {
            eprintln!("stopped leftover session {stale}");
        }
    }
    let user = UserTrace::new()
        .named("ParcelEtwSpikeDns".into())
        .enable(dns)
        .enable(modern)
        .start_and_process()
        .expect("DNS trace failed to start");
    let kernel = KernelTrace::new()
        .named("ParcelEtwSpike".into())
        .enable(net)
        .start_and_process()
        .inspect_err(|e| eprintln!("kernel logger not started: {e:?}"))
        .ok();

    let before = adapter_counters();
    println!("capturing {secs} s: browse, stream a video, download something...");
    std::thread::sleep(Duration::from_secs(secs));
    let after = adapter_counters();

    if let Some(kernel) = kernel {
        kernel.stop().expect("stopping kernel trace");
    }
    user.stop().expect("stopping DNS trace");
    report(&stats.lock().unwrap(), before, after, secs);
}
