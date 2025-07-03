//! Device struct and methods for component communication
use core::future::Future;

use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embedded_cfu_protocol::components::{CfuComponentInfo, CfuComponentStorage, CfuComponentTraits};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_cfu_protocol::{CfuWriter, CfuWriterError};
use heapless::Vec;

use super::CfuError;
use crate::GlobalRawMutex;
use crate::cfu::route_request;
use crate::intrusive_list;

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
    /// Contains an offer for the component to evaluate
    GiveOffer(FwUpdateOffer),
    /// Contains bytes for an accepted fw offer
    GiveContent(FwUpdateContentCommand),
    /// Request for component to prepare itself for an update
    PrepareComponentForUpdate,
    /// Request for component to execute any logic needed to finalize update
    FinalizeUpdate,
}

/// CFU Response types and necessary data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    state: Mutex<GlobalRawMutex, InternalState>,
    request: Channel<GlobalRawMutex, RequestData, DEVICE_CHANNEL_SIZE>,
    response: Channel<GlobalRawMutex, InternalResponseData, DEVICE_CHANNEL_SIZE>,
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

    /// Send a request to this device
    pub async fn send_request(&self, request: RequestData) {
        self.request.send(request).await;
    }

    /// Sends a request to this device and returns a response
    pub async fn execute_device_request(&self, request: RequestData) -> Result<InternalResponseData, CfuProtocolError> {
        self.send_request(request).await;
        Ok(self.wait_response().await)
    }

    /// Wait for a request
    pub async fn wait_request(&self) -> RequestData {
        self.request.receive().await
    }

    /// Send a response
    pub async fn send_response(&self, response: InternalResponseData) {
        self.response.send(response).await;
    }

    /// Waits for a response
    pub async fn wait_response(&self) -> InternalResponseData {
        self.response.receive().await
    }
}

/// Example for CFU Component
pub struct CfuComponentDefault<W: CfuWriter> {
    device: CfuDevice,
    is_dual_bank: bool,
    is_primary: bool,
    storage_offset: usize,
    subcomponents: [Option<ComponentId>; MAX_SUBCMPT_COUNT],
    writer: Mutex<GlobalRawMutex, W>,
}

impl<W: CfuWriter + Default> Default for CfuComponentDefault<W> {
    fn default() -> Self {
        Self::new(1, false, [None; MAX_SUBCMPT_COUNT], W::default())
    }
}

impl<W: CfuWriter> CfuDeviceContainer for CfuComponentDefault<W> {
    fn get_cfu_component_device(&self) -> &CfuDevice {
        &self.device
    }
}

impl<W: CfuWriter> CfuComponentDefault<W> {
    /// Constructor
    pub fn new(
        id: ComponentId,
        is_primary: bool,
        subcomponents: [Option<ComponentId>; MAX_SUBCMPT_COUNT],
        writer: W,
    ) -> Self {
        Self {
            device: CfuDevice::new(id),
            is_dual_bank: false,
            is_primary,
            storage_offset: 0,
            subcomponents,
            writer: Mutex::new(writer),
        }
    }
    /// wait for a request and process it
    pub async fn process_request(&self) -> Result<(), CfuError> {
        match self.device.wait_request().await {
            RequestData::FwVersionRequest => {
                let fwv = self.get_fw_version().await.map_err(CfuError::ProtocolError)?;
                let dev_inf = FwVerComponentInfo::new(fwv, self.get_component_id());
                let mut comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
                let mut component_count = 1;
                if self.is_primary_component() && self.get_subcomponents()[0].is_some() {
                    let arr = self
                        .get_subcomponents()
                        .iter()
                        .map(|x| x.unwrap_or_default())
                        .collect::<Vec<ComponentId, MAX_SUBCMPT_COUNT>>();
                    component_count += arr.len();
                    for (index, id) in arr.iter().enumerate() {
                        //info!("Forwarding GetFwVersion command to sub-component: {}", id);
                        if let InternalResponseData::FwVersionResponse(fwv) =
                            route_request(*id, RequestData::FwVersionRequest).await?
                        {
                            // adding 1 here is safe because MAX_CMPT_COUNT is 1 more than MAX_SUBCMPT_COUNT
                            comp_info[index + 1] = fwv.component_info[0];
                        } else {
                            /*error!(
                                "Failed to get firmware version from sub-component: {}, adding dummy info to list",
                                id
                            );*/
                            comp_info[index + 1] = FwVerComponentInfo::new(FwVersion::default(), index as u8 + 1);
                        }
                    }
                }

                let resp = GetFwVersionResponse {
                    header: GetFwVersionResponseHeader::new(
                        component_count as u8,
                        GetFwVerRespHeaderByte3::NoSpecialFlags,
                    ),
                    component_info: comp_info,
                };
                self.device
                    .send_response(InternalResponseData::FwVersionResponse(resp))
                    .await;
            }
            RequestData::PrepareComponentForUpdate => {
                self.storage_prepare()
                    .await
                    .map_err(|_| CfuError::ProtocolError(CfuProtocolError::BadResponse))?;
            }
            RequestData::GiveOffer(buf) => {
                // accept any and all offers regardless of what version it is
                if buf.component_info.component_id == self.get_component_id() {
                    let resp = FwUpdateOfferResponse::new_accept(HostToken::Driver);
                    self.device
                        .send_response(InternalResponseData::OfferResponse(resp))
                        .await;
                }
            }
            RequestData::GiveContent(buf) => {
                let offset = buf.header.firmware_address as usize;
                self.writer
                    .lock()
                    .await
                    .cfu_write(Some(offset), &buf.data)
                    .await
                    .map_err(|e| CfuError::ProtocolError(CfuProtocolError::WriterError(e)))?;
            }
            RequestData::FinalizeUpdate => {}
        }
        Ok(())
    }
}

impl<W: CfuWriter> CfuComponentInfo for CfuComponentDefault<W> {
    fn get_component_id(&self) -> ComponentId {
        self.device.component_id()
    }
    fn get_fw_version(&self) -> impl Future<Output = Result<FwVersion, CfuProtocolError>> {
        default_get_fw_version()
    }
    fn is_offer_valid(&self) -> impl Future<Output = Result<OfferStatus, (OfferStatus, OfferRejectReason)>> {
        default_is_offer_valid()
    }
    fn is_dual_bank(&self) -> bool {
        self.is_dual_bank
    }
    fn is_primary_component(&self) -> bool {
        self.is_primary
    }
    fn get_subcomponents(&self) -> [Option<ComponentId>; MAX_SUBCMPT_COUNT] {
        self.subcomponents
    }
}

impl<W: CfuWriter> CfuWriter for CfuComponentDefault<W> {
    async fn cfu_write(&self, mem_offset: Option<usize>, data: &[u8]) -> Result<(), CfuWriterError> {
        self.writer.lock().await.cfu_write(mem_offset, data).await
    }
    async fn cfu_write_read(
        &self,
        mem_offset: Option<usize>,
        data: &[u8],
        read: &mut [u8],
    ) -> Result<(), CfuWriterError> {
        self.writer.lock().await.cfu_write_read(mem_offset, data, read).await
    }

    async fn cfu_read(&self, mem_offset: Option<usize>, read: &mut [u8]) -> Result<(), CfuWriterError> {
        self.writer.lock().await.cfu_read(mem_offset, read).await
    }
}

impl<W: CfuWriter> CfuComponentStorage for CfuComponentDefault<W> {
    fn get_storage_offset(&self) -> usize {
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

async fn default_is_offer_valid() -> Result<OfferStatus, (OfferStatus, OfferRejectReason)> {
    Err((OfferStatus::Reject, OfferRejectReason::OldFw))
}
async fn default_get_fw_version() -> Result<FwVersion, CfuProtocolError> {
    Ok(FwVersion::default())
}

impl<W: CfuWriter + Default> CfuComponentTraits for CfuComponentDefault<W> {}

/// Example Wrapper for CFU Component
/// Takes type which implements `CFUComponentTraits` and `CfuDeviceContainer`
/// for message passing and cfu functionality and wraps it in a `RefCell`
/// for interior mutability
pub struct CfuWrapperDefault<C: CfuComponentTraits + CfuDeviceContainer + 'static> {
    /// internal reference to the component
    pub component: &'static C,
}

impl<C: CfuComponentTraits + CfuDeviceContainer + 'static> CfuWrapperDefault<C> {
    /// Constructor
    pub fn new(component: &'static C) -> Self {
        Self { component }
    }
}
