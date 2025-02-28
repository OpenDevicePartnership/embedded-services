#![no_std]
use embedded_cfu_protocol::protocol_definitions::CfuProtocolError;

pub mod client;
pub mod host;

pub enum CfuError {
    BadImage,
    ProtocolError(CfuProtocolError),
}
