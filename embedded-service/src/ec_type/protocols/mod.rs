//! Primitive serde and helper fns for protocols used by the EC.

/// ACPI (Advanced Configuration and Power Interface).
pub mod acpi;

/// ODP Specific Debug Protocol.
pub mod debug;

/// MCTP (Management Component Transport Protocol).
#[allow(clippy::indexing_slicing)] //panic safety: no external client, not being deployed
#[allow(clippy::unwrap_used)] //panic safety: no external client, not being deployed
pub mod mctp;

/// MTPF (Modern Thermal and Power Framework).
pub mod mptf;
