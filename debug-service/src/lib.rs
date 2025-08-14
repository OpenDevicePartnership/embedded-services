//! Debug service that captures defmt frames, buffers them, and forwards them over eSPI.
//!
//! - A global defmt logger writes bytes into a circular buffer (see `circular_buffer.rs`).
//! - `defmt_bytes_send_task` pulls framed bytes and enqueues OOB-sized chunks onto an
//!   in-process channel destined for the eSPI layer.
//! - The service also implements a simple comms request path: when it receives
//!   `DebugRxMessage::GetDataBuffer` via `comms::MailboxDelegate::receive`, it drains any
//!   currently available frames from the ring and pushes them to the same eSPI channel.
//! - A helper task can emit a small "DATA_READY" marker whenever frames are committed, which
//!   the mock eSPI service uses to know when to pull.
#![no_std]

use defmt::{error, info};

// Export transport module and types
pub mod transport;
pub use transport::get_debug_channel_receiver;
pub use transport::{DebugTransport, TransportError};

// Mock eSPI service module (a local consumer of the eSPI channel for development/testing)
pub mod mock_espi_service;
// Re-export internals from the circular buffer module for external users
mod defmt_ring_logger;
pub use defmt_ring_logger::{Queue, debug_data_available_signal, defmt_bytes_send_task_impl, get_buffer_consumer};

#[derive(Clone)]
pub struct DebugMsgComms<'a> {
    /// Shared ref to a buffer
    pub payload: embedded_services::buffer::SharedRef<'a, u8>,
    /// Size of payload
    pub payload_len: usize,
    /// Endpoint ID
    pub endpoint: embedded_services::comms::EndpointID,
}

/// Default embassy task with automatic transport selection based on enabled features
/// Transport priority: USB > eSPI > UART > NoOp
#[embassy_executor::task]
pub async fn defmt_bytes_send_task() {
    info!("Spawning defmt_bytes_send_task");
    let transport = transport::create_default_transport();
    defmt_bytes_send_task_impl(transport).await;
}

use embassy_sync::{once_lock::OnceLock, signal::Signal};
use embedded_services::GlobalRawMutex;
use embedded_services::comms::{self, EndpointID};

pub struct Service {
    pub endpoint: comms::Endpoint,
    // Signal to trigger buffer drain when GET_DATA_BUFFER is received over comms
    request_signal: Signal<GlobalRawMutex, ()>,
}

impl Service {
    pub fn new() -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::Internal(comms::Internal::Debug)),
            request_signal: Signal::new(),
        }
    }

    pub async fn process(&self) {
        // Wait for a request from the mock-eSPI service to flush data
        self.request_signal.wait().await;

        defmt::debug!("Debug service: GET_DATA_BUFFER received; draining defmt buffer");

        use crate::get_buffer_consumer;
        use crate::transport::espi::{EspiDebugMessage, get_debug_channel_sender};

        let sender = get_debug_channel_sender();
        let consumer = get_buffer_consumer();

        let mut drained_frames: u32 = 0;
        let mut drained_bytes: usize = 0;

        while let Ok(frame) = consumer.read() {
            let mut remaining = frame.as_ref();
            drained_frames += 1;
            drained_bytes += remaining.len();

            // Chunk into OOB-sized packets (64 bytes)
            while !remaining.is_empty() {
                let take = remaining.len().min(64);
                let mut data: heapless::Vec<u8, 64> = heapless::Vec::new();
                let _ = data.extend_from_slice(&remaining[..take]);
                remaining = &remaining[take..];

                let msg = EspiDebugMessage { data, port: 0 };
                if sender.try_send(msg).is_err() {
                    defmt::warn!("Mock eSPI channel full; dropping chunk");
                }
            }

            frame.release();
        }

        defmt::debug!(
            "Debug service: drained {} frames ({} bytes) into eSPI channel",
            drained_frames,
            drained_bytes
        );
    }
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(req) = message.data.get::<DebugRxMessage>() {
            match req {
                DebugRxMessage::GetDataBuffer => {
                    // Signal the processing task to drain and forward data
                    self.request_signal.signal(());
                    return Ok(());
                }
            }
        }

        Err(comms::MailboxDelegateError::MessageNotFound)
    }
}

// Service to register the debug endpoint and handle GET_DATA_BUFFER requests
#[embassy_executor::task]
pub async fn debug_service_task() {
    info!("Starting debug service task");
    static SERVICE: OnceLock<Service> = OnceLock::new();
    let service = SERVICE.get_or_init(Service::default);

    if comms::register_endpoint(service, &service.endpoint).await.is_err() {
        error!("Failed to register debug service endpoint");
        return;
    }

    loop {
        service.process().await;
    }
}

#[derive(Copy, Clone, Debug, defmt::Format)]
pub enum DebugTxMessage {
    DataReady,
    ResponseDataBuffer,
}

/// Requests handled by the debug service over the comms bus.
#[derive(Copy, Clone, Debug, defmt::Format)]
pub enum DebugRxMessage {
    /// Ask the debug service to flush any available defmt bytes to the mock eSPI service.
    GetDataBuffer,
}

#[embassy_executor::task]
pub async fn debug_service_data_ready_task() {
    info!("Starting mock data ready task");
    static SERVICE: OnceLock<Service> = OnceLock::new();
    let debug_service = SERVICE.get_or_init(Service::default);

    loop {
        info!("Debug service data ready");
        debug_service
            .endpoint
            .send(
                EndpointID::External(embedded_services::comms::External::Host),
                &DebugTxMessage::DataReady,
            )
            .await
            .unwrap();
        embassy_time::Timer::after_secs(1).await;
    }
}

// Task: forward a simple DATA_READY marker to the mock eSPI service anytime new
// defmt bytes are committed to the ring buffer.
#[embassy_executor::task]
pub async fn send_data_ready_to_mock_espi() {
    use crate::transport::espi::{EspiDebugMessage, get_debug_channel_sender};

    defmt::debug!("Starting DATA_READY forwarder to mock eSPI service");
    let sender = get_debug_channel_sender();

    loop {
        // Wait until defmt bytes are committed by the logger
        debug_data_available_signal().wait().await;

        // Build a small payload and try to send without blocking
        let mut payload: heapless::Vec<u8, 64> = heapless::Vec::new();
        let _ = payload.extend_from_slice(b"DATA_READY");
        let msg = EspiDebugMessage { data: payload, port: 0 };

        match sender.try_send(msg) {
            Ok(()) => defmt::debug!("Queued DATA_READY to mock eSPI"),
            Err(_) => defmt::warn!("Mock eSPI channel full; dropping DATA_READY"),
        }
    }
}
