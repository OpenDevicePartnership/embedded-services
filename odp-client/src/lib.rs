#![no_std]
//! Transport-blind sync ODP client and server-side service-handler macro.
//!
//! Callers issue ODP requests through an [`OdpClient`] (or any other
//! [`Relay`]) without depending on a concrete transport. The transport is
//! selected at construction time and hidden behind the [`OdpTransport`]
//! trait, so swapping the underlying medium does not affect call sites.

pub use client::{OdpClient, OdpResponse, Relay};
pub use error::OdpError;
pub use loopback::LoopbackTransport;
pub use mctp_rs::odp::{OdpHeader, OdpService};
pub use serial::SerialTransport;
pub use serializable::{MessageSerializationError, SerializableMessage, SerializableResult};
pub use server::{RelayHandler, RelayHeader, RelayResponse, RelayServiceHandler, RelayServiceHandlerTypes};
pub use transport::OdpTransport;

mod client;
mod error;
mod loopback;
mod serial;
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
