//! ODP (Open Device Partnership) MCTP message type.
//!
//! Wire format defined by the ODP project. Message type byte `0x7D`.
//! Header layout: 32-bit bitfield, little-endian on the wire.

#![cfg(feature = "odp")]
