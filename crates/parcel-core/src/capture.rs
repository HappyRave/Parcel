use std::sync::mpsc::Sender;

use crate::event::Event;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("{0} is not installed")]
    NotInstalled(&'static str),
    #[error("{0} needs administrator rights")]
    NeedsAdmin(&'static str),
    #[error("{source_name}: {message}")]
    Failed {
        source_name: &'static str,
        message: String,
    },
}

/// A platform capture backend (Npcap, ETW, later macOS / Linux ones).
pub trait CaptureSource: Send {
    /// Starts capturing on the source's own threads, sending every event to `events`.
    ///
    /// # Errors
    /// When the backend is missing or can't start.
    fn start(&mut self, events: Sender<Event>) -> Result<(), CaptureError>;

    /// Stops capturing and joins the source's threads.
    fn stop(&mut self);
}
