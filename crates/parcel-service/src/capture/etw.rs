use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::Sender;

use ferrisetw::EventRecord;
use ferrisetw::native::EvntraceNativeError;
use ferrisetw::parser::Parser;
use ferrisetw::provider::{EventFilter, Provider};
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::{TraceError, UserTrace, stop_trace_by_name};
use parcel_core::capture::{CaptureError, CaptureSource};
use parcel_core::event::{Dir, Event, Flow, Proto, now_millis};

const NAME: &str = "ETW";
const SESSION: &str = "Parcel-KernelNetwork";
const KERNEL_NETWORK: &str = "7dd42a49-5329-4832-8dfd-43d979153a88";
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
        let provider = Provider::by_guid(KERNEL_NETWORK)
            .add_filter(EventFilter::ByEventIds(SEND_RECV_IDS.to_vec()))
            .add_callback(move |record: &EventRecord, locator: &SchemaLocator| {
                if let Some(event) = transfer(record, locator) {
                    // Fails only once the engine is gone, during shutdown.
                    let _ = events.send(event);
                }
            })
            .build();
        let trace = UserTrace::new()
            .named(SESSION.to_owned())
            .enable(provider)
            .start_and_process()
            .map_err(start_error)?;
        self.trace = Some(trace);
        tracing::info!("{NAME}: capturing Kernel-Network send / receive events");
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
    fn duplicate_receive_event_is_ignored() {
        assert_eq!(kind(18), None);
    }
}
