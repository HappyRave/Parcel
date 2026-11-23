use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::Sender;

use ferrisetw::EventRecord;
use ferrisetw::native::EvntraceNativeError;
use ferrisetw::parser::Parser;
use ferrisetw::provider::{EventFilter, Provider};
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::{TraceError, UserTrace, stop_trace_by_name};
use parcel_core::capture::{CaptureError, CaptureSource};
use parcel_core::event::{Dir, Event, Flow, NameSource, Proto, now_millis};

use crate::ntpath::DeviceMap;

const NAME: &str = "ETW";
const SESSION: &str = "Parcel-KernelNetwork";
const KERNEL_NETWORK: &str = "7dd42a49-5329-4832-8dfd-43d979153a88";
const DNS_CLIENT: &str = "1c95126e-7eea-49a9-a3fe-a378b03ddb4d";
/// Microsoft-Windows-Kernel-Process: event 1 is ProcessStart, under keyword `WINEVENT_KEYWORD_PROCESS`.
const KERNEL_PROCESS: &str = "22fb2cd6-0e7b-422b-a0c7-2fad1fd0e716";
const PROCESS_KEYWORD: u64 = 0x10;
const PROCESS_START: u16 = 1;
/// Send / receive events of Microsoft-Windows-Kernel-Network. Id 18 repeats id 11's bytes.
const SEND_RECV_IDS: [u16; 8] = [10, 11, 26, 27, 42, 43, 58, 59];
/// `ERROR_ACCESS_DENIED`, plain and as the HRESULT ferrisetw reports.
const ACCESS_DENIED: [i32; 2] = [5, 0x8007_0005_u32.cast_signed()];

/// Per-process send / receive calls from the Kernel-Network ETW provider. Needs admin.
#[derive(Default)]
pub struct Etw {
    trace: Option<UserTrace>,
}

fn kind(event_id: u16) -> Option<(Proto, Dir)> {
    match event_id {
        10 | 26 => Some((Proto::Tcp, Dir::Out)),
        11 | 27 => Some((Proto::Tcp, Dir::In)),
        42 | 58 => Some((Proto::Udp, Dir::Out)),
        43 | 59 => Some((Proto::Udp, Dir::In)),
        _ => None,
    }
}

/// Reads one send / receive event; `None` for loopback or an event missing a field.
fn transfer(record: &EventRecord, locator: &SchemaLocator) -> Option<Event> {
    let (proto, dir) = kind(record.event_id())?;
    let schema = locator.event_schema(record).ok()?;
    let p = Parser::create(record, &schema);
    let pid: u32 = p.try_parse("PID").ok()?;
    let bytes: u32 = p.try_parse("size").ok()?;
    // `daddr` is always the remote side, whichever way the data goes.
    let remote: IpAddr = p.try_parse("daddr").ok()?;
    let local: IpAddr = p.try_parse("saddr").ok()?;
    let rport: u16 = p.try_parse("dport").ok()?;
    let lport: u16 = p.try_parse("sport").ok()?;
    if remote.is_loopback() {
        return None;
    }
    Some(Event::Transfer {
        at: now_millis(),
        pid,
        flow: Flow {
            proto,
            // Ports arrive in network byte order.
            local: SocketAddr::new(local.to_canonical(), u16::from_be(lport)),
            remote: SocketAddr::new(remote.to_canonical(), u16::from_be(rport)),
        },
        dir,
        bytes,
    })
}

/// Names from a DNS-client query result. Misses Chrome's own lookups, so it matters mostly
/// without Npcap; with it, Npcap sees the same replies.
fn dns_names(record: &EventRecord, locator: &SchemaLocator) -> Vec<Event> {
    let Ok(schema) = locator.event_schema(record) else {
        return Vec::new();
    };
    let p = Parser::create(record, &schema);
    let (Ok(name), Ok(results)) = (
        p.try_parse::<String>("QueryName"),
        p.try_parse::<String>("QueryResults"),
    ) else {
        return Vec::new();
    };
    let at = now_millis();
    result_ips(&results)
        .map(|ip| Event::Hostname {
            at,
            ip,
            name: name.clone(),
            source: NameSource::Dns,
        })
        .collect()
}

/// `QueryResults` is `;`-separated: addresses, plus entries like `type:  5 cname.example;`.
fn result_ips(results: &str) -> impl Iterator<Item = IpAddr> + '_ {
    results
        .split(';')
        .filter_map(|t| t.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
}

/// A process start with its image as a drive path; `None` when the path isn't on a known drive.
fn process_start(
    record: &EventRecord,
    locator: &SchemaLocator,
    drives: &DeviceMap,
) -> Option<Event> {
    let schema = locator.event_schema(record).ok()?;
    let p = Parser::create(record, &schema);
    let pid: u32 = p.try_parse("ProcessID").ok()?;
    let image: String = p.try_parse("ImageName").ok()?;
    Some(Event::ProcessStart {
        at: now_millis(),
        pid,
        image: drives.to_dos(&image)?,
    })
}

fn start_error(e: TraceError) -> CaptureError {
    match e {
        TraceError::EtwNativeError(EvntraceNativeError::IoError(io))
            if io
                .raw_os_error()
                .is_some_and(|c| ACCESS_DENIED.contains(&c)) =>
        {
            CaptureError::NeedsAdmin(NAME)
        }
        other => CaptureError::Failed {
            source_name: NAME,
            message: format!("{other:?}"),
        },
    }
}

impl CaptureSource for Etw {
    fn start(&mut self, events: Sender<Event>) -> Result<(), CaptureError> {
        // A killed service leaves its session running, and starting a new one then fails. The
        // result says nothing useful: ferrisetw reports Ok even when there was no such session.
        let _ = stop_trace_by_name(SESSION);
        // Sends fail only once the engine is gone, during shutdown.
        let net = Provider::by_guid(KERNEL_NETWORK)
            .add_filter(EventFilter::ByEventIds(SEND_RECV_IDS.to_vec()))
            .add_callback({
                let events = events.clone();
                move |record: &EventRecord, locator: &SchemaLocator| {
                    if let Some(event) = transfer(record, locator) {
                        let _ = events.send(event);
                    }
                }
            })
            .build();
        let dns = Provider::by_guid(DNS_CLIENT)
            .add_callback({
                let events = events.clone();
                move |record: &EventRecord, locator: &SchemaLocator| {
                    for event in dns_names(record, locator) {
                        let _ = events.send(event);
                    }
                }
            })
            .build();
        let drives = DeviceMap::current();
        let processes = Provider::by_guid(KERNEL_PROCESS)
            .any(PROCESS_KEYWORD)
            .add_filter(EventFilter::ByEventIds(vec![PROCESS_START]))
            .add_callback(move |record: &EventRecord, locator: &SchemaLocator| {
                if let Some(event) = process_start(record, locator, &drives) {
                    let _ = events.send(event);
                }
            })
            .build();
        let trace = UserTrace::new()
            .named(SESSION.to_owned())
            .enable(net)
            .enable(dns)
            .enable(processes)
            .start_and_process()
            .map_err(start_error)?;
        self.trace = Some(trace);
        tracing::info!("{NAME}: capturing transfers, DNS-client names and process starts");
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(trace) = self.trace.take()
            && let Err(e) = trace.stop()
        {
            tracing::error!("{NAME}: stopping {SESSION}: {e:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_ids_map_to_protocol_and_direction() {
        let kinds: Vec<_> = SEND_RECV_IDS.iter().map(|&id| kind(id)).collect();
        assert_eq!(
            kinds,
            vec![
                Some((Proto::Tcp, Dir::Out)),
                Some((Proto::Tcp, Dir::In)),
                Some((Proto::Tcp, Dir::Out)),
                Some((Proto::Tcp, Dir::In)),
                Some((Proto::Udp, Dir::Out)),
                Some((Proto::Udp, Dir::In)),
                Some((Proto::Udp, Dir::Out)),
                Some((Proto::Udp, Dir::In)),
            ]
        );
    }

    #[test]
    fn dns_results_keep_only_addresses() {
        let ips: Vec<IpAddr> =
            result_ips("type:  5 www.l.google.com;142.250.74.4;::ffff:142.250.74.5;2a00:1450::4;")
                .collect();
        assert_eq!(
            ips,
            [
                "142.250.74.4".parse::<IpAddr>().unwrap(),
                "142.250.74.5".parse().unwrap(),
                "2a00:1450::4".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn duplicate_receive_event_is_ignored() {
        assert_eq!(kind(18), None);
    }
}
