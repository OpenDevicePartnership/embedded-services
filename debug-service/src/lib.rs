#![no_std]
mod debug_service;
mod defmt_ring_logger;

pub use debug_service::*;
pub use defmt_ring_logger::defmt_to_host_task;
