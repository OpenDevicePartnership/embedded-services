#![no_std]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

mod espi_service;
mod mctp;
pub mod task;

pub use espi_service::*;
