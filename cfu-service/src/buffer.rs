//! Module that can buffer CFU content
//! This allows prompt responses to content requests even if the component is busy

use core::{cell::RefCell, future::pending};

use embassy_futures::select::{select3, Either3};
use embassy_sync::channel::{DynamicReceiver, DynamicSender};
use embassy_time::{with_timeout, Duration, TimeoutError};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::{
    cfu::{
        self,
        component::{CfuDevice, InternalResponse, InternalResponseData, RequestData},
    },
    error, intrusive_list,
    ipc::deferred::RequestId,
    trace,
};

/// Internal state for [`Buffer`]
#[derive(Copy, Clone, Default)]
struct State {
    /// Component response that arrived outside of the timeout window
    pending_response: Option<InternalResponseData>,
    /// Whether the component is busy processing a request
    component_busy: bool,
}

/// Buffer config
#[derive(Copy, Clone, Default)]
pub struct Config {
    /// Maximum amount of time to wait for a request to complete
    buffer_timeout: Duration,
}

impl Config {
    /// Create a new config with a timeout
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            buffer_timeout: timeout,
        }
    }
}

/// CFU buffer
///
/// This will accept and buffer CFU content if the target component is busy.
/// This means that errors that happen while processing buffered content
/// may not reach the CFU host. Devices should be prepared to handle this
/// and may choose to pad the end of the update with dummy data.
pub struct Buffer<'a> {
    /// CFU device
    cfu_device: CfuDevice,
    /// Internal state
    state: RefCell<State>,
    /// Component ID to buffer requests for
    buffered_id: ComponentId,
    /// Sender for the buffer
    buffer_sender: DynamicSender<'a, FwUpdateContentCommand>,
    /// Receiver for the buffer
    buffer_receiver: DynamicReceiver<'a, FwUpdateContentCommand>,
    /// Configuration for the buffer
    config: Config,
}

pub enum Event {
    /// Content request from the host
    CfuRequest(RequestData),
    /// Available buffered content
    BufferedContent(FwUpdateContentCommand),
    /// Response from the buffered component
    ComponentResponse(InternalResponseData),
}

impl<'a> Buffer<'a> {
    /// Create a new Buffer
    ///
    /// The buffer receives requests send to external_id and forwards them to buffered_id.
    pub fn new(
        external_id: ComponentId,
        buffered_id: ComponentId,
        buffer_sender: DynamicSender<'a, FwUpdateContentCommand>,
        buffer_receiver: DynamicReceiver<'a, FwUpdateContentCommand>,
        config: Config,
    ) -> Self {
        Self {
            cfu_device: CfuDevice::new(external_id),
            state: RefCell::new(Default::default()),
            buffered_id,
            buffer_sender,
            buffer_receiver,
            config,
        }
    }

    /// Create a new invalid FW version response
    fn create_invalid_fw_version_response(&self) -> InternalResponseData {
        let dev_inf = FwVerComponentInfo::new(FwVersion::new(0xffffffff), self.cfu_device.component_id());
        let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
        InternalResponseData::FwVersionResponse(GetFwVersionResponse {
            header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
            component_info: comp_info,
        })
    }

    /// Create a content rejection response
    fn create_content_rejection(sequence: u16) -> InternalResponseData {
        InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
            sequence,
            CfuUpdateContentResponseStatus::ErrorInvalid,
        ))
    }

    /// Process a fw version request
    async fn process_get_fw_version(&self) -> InternalResponseData {
        if let Ok(InternalResponseData::FwVersionResponse(mut response)) =
            cfu::route_request(self.buffered_id, RequestData::FwVersionRequest).await
        {
            // Update the component ID in the response to match our external ID
            response.component_info[0].component_id = self.cfu_device.component_id();
            InternalResponseData::FwVersionResponse(response)
        } else {
            error!("Failed to get FW version for device {}", self.buffered_id);
            self.create_invalid_fw_version_response()
        }
    }

    /// Process a give offer request
    async fn process_give_offer(&self, offer: &FwUpdateOffer) -> InternalResponseData {
        let mut offer = *offer;
        offer.component_info.component_id = self.buffered_id;
        if let Ok(response @ InternalResponseData::OfferResponse(_)) =
            cfu::route_request(self.buffered_id, RequestData::GiveOffer(offer)).await
        {
            response
        } else {
            error!("Failed to give offer for device {}", self.buffered_id);
            InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_with_failure(
                HostToken::Driver,
                OfferRejectReason::InvalidComponent,
                OfferStatus::Reject,
            ))
        }
    }

    /// Process update content
    async fn process_give_content(&self, state: &mut State, content: &FwUpdateContentCommand) -> InternalResponseData {
        // Clear out any pending response if this is a new FW update
        if content.header.flags & FW_UPDATE_FLAG_FIRST_BLOCK != 0 {
            state.pending_response = None;
        }

        let mut have_id = false;
        let mut req_id: RequestId = RequestId::default();
        if state.component_busy {
            // Buffer the content if the component is busy
            // If the buffer is full, this will block until space is available
            trace!("Component is busy, buffering content");
            self.buffer_sender.send(*content).await;
        } else {
            // Buffered component can accept new content, send it
            match cfu::send_device_request(self.buffered_id, RequestData::GiveContent(*content)).await {
                Ok(id) => {
                    have_id = true;
                    req_id = id;
                }
                Err(e) => {
                    error!(
                        "Failed to send content to buffered component {:?}: {:?}",
                        self.buffered_id, e
                    );
                    return Self::create_content_rejection(content.header.sequence_num);
                }
            }
        }

        // Wait for a response from the buffered component
        let fut = if have_id {
            cfu::wait_device_response(self.buffered_id, Some(req_id))
        } else {
            cfu::wait_device_response(self.buffered_id, None)
        };
        match with_timeout(self.config.buffer_timeout, fut).await {
            Err(TimeoutError) => {
                // Component didn't respond in time
                state.component_busy = true;

                // Have most recent response from component, send that instead
                if let Some(response) = state.pending_response.take() {
                    if let InternalResponseData::ContentResponse(mut response) = response {
                        // Update the sequence number to pretend it's for this content.
                        trace!("Using pending response: {:?}", response);
                        response.sequence = content.header.sequence_num;
                        InternalResponseData::ContentResponse(response)
                    } else {
                        // This should never happen and means that the component sent an invalid response
                        // But send it on anyway
                        error!("Pending response is not a content response: {:?}", response);
                        response
                    }
                } else {
                    // Otherwise just accept the content
                    trace!("Buffered component timed out, sending accept response");
                    InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::Success,
                    ))
                }
            }
            Ok(response) => {
                trace!("Buffered component responded");
                state.component_busy = false;
                match response.unwrap() {
                    Ok(InternalResponseData::ContentResponse(mut response)) => {
                        response.sequence = content.header.sequence_num;
                        InternalResponseData::ContentResponse(response)
                    }
                    Ok(response) => response,
                    Err(e) => {
                        // Couldn't get any response from the buffered component, send a rejection
                        error!(
                            "Failed to get response from buffered component {:?}: {:?}",
                            self.buffered_id, e
                        );
                        Self::create_content_rejection(content.header.sequence_num)
                    }
                }
            }
        }
    }

    /// Wait for buffered content
    ///
    /// If the component is busy, this will wait indefinitely since the component will not be able to process
    async fn wait_buffered_content(&self, is_busy: bool) -> FwUpdateContentCommand {
        if is_busy {
            let () = pending().await;
            unreachable!();
        } else {
            self.buffer_receiver.receive().await
        }
    }

    /// Wait for an event
    pub async fn wait_event(&self) -> Event {
        let is_busy = self.state.borrow().component_busy;
        match select3(
            // Wait for a buffered content request
            self.wait_buffered_content(is_busy),
            // Wait for a request from the host
            self.cfu_device.receive(),
            // Wait for response from the buffered component
            cfu::wait_device_response(self.buffered_id, None),
        )
        .await
        {
            Either3::First(content) => {
                trace!("Buffered content received: {:?}", content);
                Event::BufferedContent(content)
            }
            Either3::Second(request) => {
                trace!("Request received: {:?}", request.command);
                Event::CfuRequest(request.command.data)
            }
            Either3::Third(response) => {
                if let Ok(Ok(response)) = response {
                    trace!("Response received: {:?}", response);
                    Event::ComponentResponse(response)
                } else {
                    error!("Failed to get response from buffered component: {:?}", response);
                    Event::ComponentResponse(Self::create_content_rejection(0))
                }
            }
        }
    }

    /// Top-level event processing function
    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn process(&self, event: Event) -> Option<InternalResponseData> {
        let mut state = self.state.borrow_mut();
        match event {
            Event::CfuRequest(request) => Some(self.process_request(&mut state, request).await),
            Event::BufferedContent(content) => {
                // Send the buffered content to the component
                // Don't need to wait for a response here, the response will be caught later by either [`wait_event`] or [`process_give_content`]
                if let Err(e) = cfu::send_device_request(self.buffered_id, RequestData::GiveContent(content)).await {
                    error!(
                        "Failed to send content to buffered component {:?}: {:?}",
                        self.buffered_id, e
                    );
                    Some(Self::create_content_rejection(content.header.sequence_num))
                } else {
                    state.component_busy = true;
                    None
                }
            }
            Event::ComponentResponse(response) => {
                // Store the response for the next content request
                state.pending_response = Some(response);
                state.component_busy = false;
                None
            }
        }
    }

    /// Process a CFU message and produce a response
    async fn process_request(&self, state: &mut State, request: RequestData) -> InternalResponseData {
        match request {
            RequestData::FwVersionRequest => {
                trace!("Got FwVersionRequest");
                self.process_get_fw_version().await
            }
            RequestData::GiveOffer(offer) => {
                trace!("Got GiveOffer");
                self.process_give_offer(&offer).await
            }
            RequestData::GiveContent(content) => {
                trace!("Got GiveContent");
                self.process_give_content(state, &content).await
            }
            RequestData::FinalizeUpdate => {
                trace!("Got FinalizeUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::PrepareComponentForUpdate => {
                trace!("Got PrepareComponentForUpdate");
                InternalResponseData::ComponentPrepared
            }
        }
    }

    /// Send a response to the CFU message
    pub async fn send_response(&self, response: InternalResponse, request_id: RequestId) {
        self.cfu_device.send_response(request_id, response).await
    }

    /// Register the buffer with all relevant services
    pub async fn register(&'static self) -> Result<(), intrusive_list::Error> {
        cfu::register_device(&self.cfu_device).await
    }
}
