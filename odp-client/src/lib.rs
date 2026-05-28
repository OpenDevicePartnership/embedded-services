#![no_std]
//! Sync ODP client + transport abstraction.
//!
//! Consumers depend on this crate, NOT on `mctp-rs` directly. The wire format
//! lives in `mctp_rs::odp` and is re-exported here for convenience.

pub use mctp_rs::odp::{ODP_MESSAGE_TYPE, OdpHeader, OdpMessage, OdpService};
pub use error::OdpError;
pub use transport::OdpTransport;
pub use mctp_serial::MctpSerialTransport;
pub use loopback::LoopbackTransport;

mod client;
mod error;
mod loopback;
mod mctp_serial;
mod server;
mod transport;
