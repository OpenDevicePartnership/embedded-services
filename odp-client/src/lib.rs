#![no_std]
//! Sync ODP client + transport abstraction.
//!
//! Consumers depend on this crate, NOT on `mctp-rs` directly. The wire format
//! lives in `mctp_rs::odp` and is re-exported here for convenience.

pub use client::{OdpClient, OdpResponse, Relay};
pub use error::OdpError;
pub use loopback::LoopbackTransport;
pub use mctp_rs::odp::{ODP_MESSAGE_TYPE, OdpHeader, OdpMessage, OdpService};
pub use mctp_serial::MctpSerialTransport;
pub use serializable::{MessageSerializationError, SerializableMessage, SerializableResult};
pub use server::{MctpError, RelayHandler, RelayHeader, RelayResponse, RelayServiceHandler, RelayServiceHandlerTypes};
pub use transport::OdpTransport;

mod client;
mod error;
mod loopback;
mod mctp_serial;
pub mod serializable;
pub mod server;
mod transport;

/// Hidden re-exports used by the [`impl_odp_relay_handler`] macro.
/// Not part of the public API — do not depend on these directly.
#[doc(hidden)]
pub mod _macro_internal {
    pub use bitfield;
    pub use mctp_rs;
    pub use paste;
}
