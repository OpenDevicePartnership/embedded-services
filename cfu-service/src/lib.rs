#![no_std]
use embedded_cfu_protocol::protocol_definitions::CfuProtocolError;

pub enum CfuError {
    BadImage,
    ProtocolError(CfuProtocolError),
}
