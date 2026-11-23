//! The wire protocol between `parcel-service` and the Parcel app: messages, framing and the
//! named pipe.

pub mod messages;

#[cfg(feature = "pipe")]
pub mod frame;
#[cfg(all(feature = "pipe", windows))]
pub mod pipe;
