//! Device struct and methods for component communication
use core::future::Future;
use core::ops::DerefMut;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embedded_cfu_protocol::components::{CfuComponentInfo, CfuComponentStorage, CfuComponentTraits};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_cfu_protocol::{CfuWriter, CfuWriterDefault, CfuWriterError};
use heapless::Vec;

use crate::cfu::send_request;
use crate::intrusive_list;

use super::CfuError;

/// Component internal update state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ComponentState {
    /// Component not currently processing an update
    Idle,
    /// Component is ready to receive an offer,
    Ready,
    /// Component is busy with an update
    Busy,
    /// Component has received all new fw bytes and needs finalization logic
    FinalizingUpdate,
}

/// Internal device state for power policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InternalState {
    /// Current state of the device
    pub state: ComponentState,
    /// Current consumer capability
    pub waiting_on_subs: bool,
}

impl InternalState {
    /// Constructor for a given `state`
    pub fn new(state: ComponentState) -> Self {
        Self {
            state,
            waiting_on_subs: false,
        }
    }
    /// Constructor that uses given values for both `state` and `waiting_on_subs`
    pub fn new_with_subcomponent_info(state: ComponentState, waiting_on_subs: bool) -> Self {
        Self { state, waiting_on_subs }
    }
}

impl Default for InternalState {
    fn default() -> Self {
        Self::new(ComponentState::Idle)
    }
}

/// CFU Request types and necessary data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RequestData {
    /// Request for component's current FW version
    FwVersionRequest,
    /// Request from a Primary component for its subcomponents fw versions
    PrimaryNeedsSubcomponentFwVersion([ComponentId; MAX_CMPT_COUNT - 1]),
    /// Contains an offer for the component to evaluate
    GiveOffer(FwUpdateOfferCommand),
    /// Contains bytes for an accepted fw offer
    GiveContent(FwUpdateContentCommand<&'static [u8]>),
    /// Request for component to prepare itself for an update
    PrepareComponentForUpdate,
    /// Request for component to execute any logic needed to finalize update
    FinalizeUpdate,
}

/// CFU Response types and necessary data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalResponseData {
    /// Component is mid-update and auto-rejecting offers
    ComponentBusy,
    /// Full response struct for getfwversion request
    FwVersionResponse(GetFwVersionResponse),
    /// Full response struct for fwupdateoffer request
    OfferResponse(FwUpdateOfferResponse),
    /// Response for each packet of fw update
    ContentResponse(FwUpdateContentResponse),
    /// Component has sub-components which also need to prep for fw update
    PrimaryNeedsSubcomponentsPrepared([ComponentId; MAX_CMPT_COUNT - 1]),
    /// Subcomponent versions
    SubcomponentFwVersionResponse([FwVerComponentInfo; MAX_CMPT_COUNT - 1]),
    /// Component is ready to receive offers
    ComponentPrepared,
}

/// Channel size for device requests
pub const DEVICE_CHANNEL_SIZE: usize = 1;

/// CfuDevice struct
/// Can be inserted in an intrusive-list+
pub struct CfuDevice {
    node: intrusive_list::Node,
    component_id: ComponentId,
    state: Mutex<NoopRawMutex, InternalState>,
    request: Channel<NoopRawMutex, RequestData, DEVICE_CHANNEL_SIZE>,
    response: Channel<NoopRawMutex, InternalResponseData, DEVICE_CHANNEL_SIZE>,
}

impl intrusive_list::NodeContainer for CfuDevice {
    fn get_node(&self) -> &intrusive_list::Node {
        &self.node
    }
}

/// Trait for any container that holds a device
pub trait CfuDeviceContainer {
    /// Get the underlying device struct
    fn get_cfu_component_device(&self) -> &CfuDevice;
}

impl CfuDeviceContainer for CfuDevice {
    fn get_cfu_component_device(&self) -> &CfuDevice {
        self
    }
}

impl CfuDevice {
    /// Constructor for CfuDevice
    pub fn new(component_id: ComponentId) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            component_id,
            state: Mutex::new(InternalState::default()),
            request: Channel::new(),
            response: Channel::new(),
        }
    }
    /// Getter for component id
    pub fn component_id(&self) -> ComponentId {
        self.component_id
    }
    /// Setter for component state
    /// Intended to be used to auto-block updates if one is in-progress
    pub async fn state(&self) -> InternalState {
        *self.state.lock().await
    }
    /// Sends a request to this device and returns a response
    pub(super) async fn execute_device_request(
        &self,
        request: RequestData,
    ) -> Result<InternalResponseData, CfuProtocolError> {
        self.request.send(request).await;
        Ok(self.response.receive().await)
    }

    /// Wait for a request
    pub async fn wait_request(&self) -> RequestData {
        self.request.receive().await
    }

    /// Send a response
    pub async fn send_response(&self, response: InternalResponseData) {
        self.response.send(response).await;
    }

    /// Internal function to set device state
    pub(super) async fn set_state(&self, new_state: InternalState) {
        let mut lock = self.state.lock().await;
        let state = lock.deref_mut();
        *state = new_state;
    }
}

/// Example wrapper for a CFU Component
pub struct CfuComponentDefaultWrapper {
    device: CfuDevice,
    is_dual_bank: bool,
    is_primary: bool,
    storage_offset: usize,
    subcomponents: Option<Vec<ComponentId, { MAX_CMPT_COUNT - 1 }>>,
}

impl Default for CfuComponentDefaultWrapper {
    fn default() -> Self {
        Self::new(1, false, None)
    }
}

impl CfuDeviceContainer for CfuComponentDefaultWrapper {
    fn get_cfu_component_device(&self) -> &CfuDevice {
        &self.device
    }
}

impl CfuComponentDefaultWrapper {
    /// Constructor
    pub fn new(
        id: ComponentId,
        is_primary: bool,
        subcomponents: Option<Vec<ComponentId, { MAX_CMPT_COUNT - 1 }>>,
    ) -> Self {
        Self {
            device: CfuDevice::new(id),
            is_dual_bank: false,
            is_primary,
            storage_offset: 0,
            subcomponents,
        }
    }
    /// wait for a request and process it
    pub async fn process_request(&self) -> Result<(), CfuError> {
        match self.device.wait_request().await {
            RequestData::FwVersionRequest => {
                let fwv = self.get_fw_version().await.map_err(CfuError::ProtocolError)?;
                let dev_inf = FwVerComponentInfo::new(fwv, self.get_component_id(), BankType::SingleBank);
                let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
                let resp = GetFwVersionResponse {
                    header: GetFwVersionResponseHeader::default(),
                    component_info: comp_info,
                };

                if self.is_primary_component() && self.get_subcomponents().is_some() {
                    let arr: [ComponentId; MAX_CMPT_COUNT - 1] =
                        self.get_subcomponents().unwrap().into_array().unwrap();
                    send_request(
                        self.get_component_id(),
                        RequestData::PrimaryNeedsSubcomponentFwVersion(arr),
                    )
                    .await?;
                } else {
                    self.device
                        .send_response(InternalResponseData::FwVersionResponse(resp))
                        .await;
                }
            }
            RequestData::PrepareComponentForUpdate => {
                self.storage_prepare()
                    .await
                    .map_err(|_| CfuError::ProtocolError(CfuProtocolError::BadResponse))?;
            }
            RequestData::GiveOffer(buf) => {
                // accept any and all offers regardless of what version it is
                if buf.component_info.component_id == self.get_component_id() {
                    let resp = FwUpdateOfferResponse::new_success(0);
                    self.device
                        .send_response(InternalResponseData::OfferResponse(resp))
                        .await;
                }
            }
            RequestData::GiveContent(buf) => {
                let offset = buf.header.firmware_address as usize;
                self.cfu_write(Some(offset), buf.data)
                    .await
                    .map_err(|e| CfuError::ProtocolError(CfuProtocolError::WriterError(e)))?;
            }
            RequestData::FinalizeUpdate => {}
            RequestData::PrimaryNeedsSubcomponentFwVersion(_ids) => {}
        }
        Ok(())
    }
}

impl CfuComponentInfo for CfuComponentDefaultWrapper {
    fn get_component_id(&self) -> ComponentId {
        self.device.component_id()
    }
    fn get_fw_version(&self) -> impl Future<Output = Result<FwVersion, CfuProtocolError>> {
        default_get_fw_version()
    }
    fn is_offer_valid(
        &self,
    ) -> impl Future<Output = Result<CfuOfferResponseStatus, (CfuOfferResponseStatus, RejectReason)>> {
        default_is_offer_valid()
    }
    fn is_dual_bank(&self) -> bool {
        self.is_dual_bank
    }
    fn is_primary_component(&self) -> bool {
        self.is_primary
    }
    fn get_subcomponents(&self) -> Option<Vec<ComponentId, { MAX_CMPT_COUNT - 1 }>> {
        self.subcomponents.clone()
    }
}

impl CfuWriter for CfuComponentDefaultWrapper {
    async fn cfu_write(&self, mem_offset: Option<usize>, data: &[u8]) -> Result<(), CfuWriterError> {
        let mockwriter = CfuWriterDefault::new();
        mockwriter.cfu_write(mem_offset, data).await
    }
    async fn cfu_write_read(
        &self,
        mem_offset: Option<usize>,
        data: &[u8],
        read: &mut [u8],
    ) -> Result<(), CfuWriterError> {
        let mockwriter = CfuWriterDefault::new();
        mockwriter.cfu_write_read(mem_offset, data, read).await
    }

    async fn cfu_read(&self, mem_offset: Option<usize>, read: &mut [u8]) -> Result<(), CfuWriterError> {
        let mockwriter = CfuWriterDefault::new();
        mockwriter.cfu_read(mem_offset, read).await
    }
}

impl CfuComponentStorage for CfuComponentDefaultWrapper {
    fn get_storage_offset<T>(&self, _args: Option<T>) -> usize {
        self.storage_offset
    }
    async fn storage_finalize(&self) -> Result<(), CfuWriterError> {
        Ok(())
    }
    async fn storage_prepare(&self) -> Result<(), CfuWriterError> {
        Ok(())
    }
    async fn storage_write(&self) -> Result<(), CfuWriterError> {
        Ok(())
    }
}

async fn default_is_offer_valid() -> Result<CfuOfferResponseStatus, (CfuOfferResponseStatus, RejectReason)> {
    Err((CfuOfferResponseStatus::ErrorNoOffer, RejectReason::VendorSpecific))
}
async fn default_get_fw_version() -> Result<FwVersion, CfuProtocolError> {
    Ok(FwVersion::default())
}

impl CfuComponentTraits for CfuComponentDefaultWrapper {}
