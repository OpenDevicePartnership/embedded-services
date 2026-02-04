#![no_std]

use embassy_sync::channel::Channel;
use embedded_cfu_protocol::client::CfuReceiveContent;
use embedded_cfu_protocol::components::CfuComponentTraits;
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::{GlobalRawMutex, comms, error, info, intrusive_list, trace};

pub mod buffer;
pub mod component;
pub mod host;
pub mod splitter;
pub mod task;

pub struct CfuClient {
    /// Cfu Client context
    context: ClientContext,
    /// Comms endpoint
    tp: comms::Endpoint,
}

impl<T, C> CfuReceiveContent<T, C, ()> for CfuClient {
    async fn process_command(&self, _args: Option<T>, _cmd: C) -> Result<(), ()> {
        trace!("CfuClient CfuReceiveContent::process_command do nothing implementation.");
        Ok(())
    }

    async fn prepare_components(
        &self,
        _args: Option<T>,
        _primary_component: impl CfuComponentTraits,
    ) -> Result<(), ()> {
        trace!("CfuClient CfuReceiveContent::prepare_components do nothing implementation.");
        Ok(())
    }
}

impl CfuClient {
    /// Create a new Cfu Client
    pub async fn new(service_storage: &'static embassy_sync::once_lock::OnceLock<CfuClient>) -> &'static Self {
        let service_storage = service_storage.get_or_init(|| Self {
            context: ClientContext::new(),
            tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Nonvol)),
        });

        service_storage.init().await;

        service_storage
    }

    async fn init(&'static self) {
        if comms::register_endpoint(self, &self.tp).await.is_err() {
            error!("Failed to register cfu endpoint");
        }
    }

    pub async fn process_request(&self) -> Result<(), CfuError> {
        let request = self.context.wait_request().await;
        //let device = self.context.get_device(request.id).await?;
        let comp = request.id;

        match request.data {
            component::RequestData::FwVersionRequest => {
                info!("Received FwVersionRequest, comp {}", comp);
                if let Ok(device) = self.context.get_device(comp) {
                    let resp = device
                        .execute_device_request(request.data)
                        .await
                        .map_err(CfuError::ProtocolError)?;

                    // TODO replace with signal to component to get its own fw version
                    //cfu::send_request(comp, RequestData::FwVersionRequest).await?;
                    match resp {
                        component::InternalResponseData::FwVersionResponse(r) => {
                            let ver = r.component_info[0].fw_version;
                            info!("got fw version {:?} for comp {}", ver, comp);
                        }
                        _ => {
                            error!("Invalid response to get fw version {:?} from comp {}", resp, comp);
                            return Err(CfuError::ProtocolError(CfuProtocolError::BadResponse));
                        }
                    }
                    self.context.send_response(resp).await;
                    return Ok(());
                }
                Err(CfuError::InvalidComponent)
            }
            component::RequestData::GiveContent(_content_cmd) => Ok(()),
            component::RequestData::GiveOffer(_offer_cmd) => Ok(()),
            component::RequestData::PrepareComponentForUpdate => Ok(()),
            component::RequestData::AbortUpdate => Ok(()),
            component::RequestData::FinalizeUpdate => Ok(()),
            component::RequestData::GiveOfferExtended(_) => {
                // Don't currently support extended offers
                self.context
                    .send_response(component::InternalResponseData::OfferResponse(
                        FwUpdateOfferResponse::new_with_failure(
                            HostToken::Driver,
                            OfferRejectReason::InvalidComponent,
                            OfferStatus::Reject,
                        ),
                    ))
                    .await;
                Ok(())
            }
            component::RequestData::GiveOfferInformation(_) => {
                // Don't currently support information offers
                self.context
                    .send_response(component::InternalResponseData::OfferResponse(
                        FwUpdateOfferResponse::new_with_failure(
                            HostToken::Driver,
                            OfferRejectReason::InvalidComponent,
                            OfferStatus::Reject,
                        ),
                    ))
                    .await;
                Ok(())
            }
        }
    }

    pub fn register_device(
        &self,
        device: &'static impl component::CfuDeviceContainer,
    ) -> Result<(), intrusive_list::Error> {
        self.context.register_device(device)
    }

    pub async fn route_request(
        &self,
        to: ComponentId,
        request: component::RequestData,
    ) -> Result<component::InternalResponseData, CfuError> {
        self.context.route_request(to, request).await
    }
}

impl comms::MailboxDelegate for CfuClient {}

/// Error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CfuError {
    /// Image did not pass validation
    BadImage,
    /// Component either doesn't exist
    InvalidComponent,
    /// Component is busy
    ComponentBusy,
    /// Component encountered a protocol error during execution
    ProtocolError(CfuProtocolError),
}

/// Request to the power policy service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Request {
    /// Component that sent this request
    pub id: ComponentId,
    /// Request data
    pub data: component::RequestData,
}

/// Cfu context
pub struct ClientContext {
    /// Registered devices
    devices: embedded_services::intrusive_list::IntrusiveList,
    /// Request to components
    request: Channel<GlobalRawMutex, Request, { component::DEVICE_CHANNEL_SIZE }>,
    /// Response from components
    response: Channel<GlobalRawMutex, component::InternalResponseData, { component::DEVICE_CHANNEL_SIZE }>,
}

impl Default for ClientContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientContext {
    pub fn new() -> Self {
        Self {
            devices: embedded_services::intrusive_list::IntrusiveList::new(),
            request: Channel::new(),
            response: Channel::new(),
        }
    }

    /// Register a device with the Cfu Client service
    fn register_device(
        &self,
        device: &'static impl component::CfuDeviceContainer,
    ) -> Result<(), intrusive_list::Error> {
        let device = device.get_cfu_component_device();
        if self.get_device(device.component_id()).is_ok() {
            return Err(intrusive_list::Error::NodeAlreadyInList);
        }

        self.devices.push(device)
    }

    /// Convenience function to send a request to the Cfu service
    pub async fn send_request(
        &self,
        from: ComponentId,
        request: component::RequestData,
    ) -> Result<component::InternalResponseData, CfuError> {
        self.request
            .send(Request {
                id: from,
                data: request,
            })
            .await;
        Ok(self.response.receive().await)
    }

    /// Convenience function to route a request to a specific component
    pub async fn route_request(
        &self,
        to: ComponentId,
        request: component::RequestData,
    ) -> Result<component::InternalResponseData, CfuError> {
        let device = self.get_device(to)?;
        device
            .execute_device_request(request)
            .await
            .map_err(CfuError::ProtocolError)
    }

    /// Send a request to the specific CFU device, but don't wait for a response
    pub async fn send_device_request(&self, to: ComponentId, request: component::RequestData) -> Result<(), CfuError> {
        let device = self.get_device(to)?;
        device.send_request(request).await;
        Ok(())
    }

    /// Wait for a response from the specific CFU device
    pub async fn wait_device_response(&self, to: ComponentId) -> Result<component::InternalResponseData, CfuError> {
        let device = self.get_device(to)?;
        Ok(device.wait_response().await)
    }

    /// Wait for a cfu request
    pub async fn wait_request(&self) -> Request {
        self.request.receive().await
    }

    /// Send a response to a cfu request
    pub async fn send_response(&self, response: component::InternalResponseData) {
        self.response.send(response).await
    }

    /// Get a device by its ID
    pub fn get_device(&self, id: ComponentId) -> Result<&'static component::CfuDevice, CfuError> {
        for device in &self.devices {
            if let Some(data) = device.data::<component::CfuDevice>() {
                if data.component_id() == id {
                    return Ok(data);
                }
            } else {
                error!("Non-device located in devices list");
            }
        }

        Err(CfuError::InvalidComponent)
    }

    /// Provides access to the device list
    pub fn devices(&self) -> &intrusive_list::IntrusiveList {
        &self.devices
    }
}
