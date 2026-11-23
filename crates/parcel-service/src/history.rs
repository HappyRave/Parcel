use std::path::Path;
use std::sync::{Arc, Mutex};

use parcel_core::event::now_millis;
use parcel_core::store::{Store, local_offset};
use parcel_ipc::messages::{Reply, Request};
use parcel_ipc::pipe::Handler;

/// Answers the app's history requests from a read-only connection beside the recorder's.
///
/// # Errors
/// When the database can't be opened.
pub fn handler(db: &Path) -> rusqlite::Result<Handler> {
    let store = Mutex::new(Store::open_read(db)?);
    Ok(Arc::new(move |request| {
        let store = store.lock().expect("history store lock");
        let result = match request {
            Request::Usage { from, to } => store
                .usage(from, to, now_millis(), local_offset())
                .map(|apps| Reply::Usage { apps }),
        };
        result.unwrap_or_else(|e| {
            tracing::error!("history query failed: {e}");
            Reply::Error {
                message: e.to_string(),
            }
        })
    }))
}
