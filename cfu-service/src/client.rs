use embedded_cfu_protocol::components::CfuComponentMockWrapper;
pub use embedded_cfu_protocol::CfuReceiveContent;

pub struct CfuClientInstanceMock {
    pub primary_cmpt: CfuComponentMockWrapper,
}

/// use default "do-nothing" implementations
impl<T, C, E: Default> CfuReceiveContent<T, C, E> for CfuClientInstanceMock {}
