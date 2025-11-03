use cfu_service::buffer::Config;
use embassy_executor::{Executor, Spawner};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::{Duration, Timer};
use embedded_services::GlobalRawMutex;
use embedded_services::{
    cfu::component::{CfuDevice, CfuDeviceContainer, InternalResponseData},
    intrusive_list,
};
use log::*;
use static_cell::StaticCell;

use embassy_sync::channel::{SendDynamicReceiver, SendDynamicSender};

use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::cfu::component::RequestData;
use embedded_services::cfu::{self, route_request};

use cfu_service::buffer::{self, Event};

/// Component ID for the CFU buffer
const CFU_BUFFER_ID: ComponentId = 0x06;

/// Component ID for the mock device
const CFU_COMPONENT0_ID: ComponentId = 0x20;

mod mock {
    use super::*;

    /// Mock CFU device
    pub struct Device {
        cfu_device: CfuDevice,
        version: FwVersion,
    }

    impl Device {
        /// Create a new mock CFU device
        pub fn new(component_id: ComponentId, version: FwVersion) -> Self {
            Self {
                cfu_device: CfuDevice::new(component_id),
                version,
            }
        }

        /// Wait for a CFU message
        pub async fn wait_request(&self) -> RequestData {
            self.cfu_device.wait_request().await
        }

        /// Process a CFU message and produce a response
        pub async fn process_request(&self, request: RequestData) -> InternalResponseData {
            match request {
                RequestData::FwVersionRequest => {
                    info!("Got FwVersionRequest");
                    let dev_inf = FwVerComponentInfo::new(self.version, self.cfu_device.component_id());
                    let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
                    InternalResponseData::FwVersionResponse(GetFwVersionResponse {
                        header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
                        component_info: comp_info,
                    })
                }
                RequestData::GiveOffer(offer) => {
                    trace!("Got GiveOffer");
                    if offer.component_info.component_id != self.cfu_device.component_id() {
                        InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_with_failure(
                            HostToken::Driver,
                            OfferRejectReason::InvalidComponent,
                            OfferStatus::Reject,
                        ))
                    } else {
                        InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_accept(HostToken::Driver))
                    }
                }
                RequestData::GiveContent(content) => {
                    if content.header.flags & FW_UPDATE_FLAG_LAST_BLOCK != 0 {
                        // Take 5000 ms to finish the update
                        info!("Finishing update, taking 5000 ms");
                        embassy_time::Timer::after_millis(5000).await;
                        info!("Device initialized");
                    }

                    InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::Success,
                    ))
                }
                RequestData::FinalizeUpdate => {
                    trace!("Got FinalizeUpdate");
                    InternalResponseData::ComponentPrepared
                }
                RequestData::PrepareComponentForUpdate => {
                    trace!("Got PrepareComponentForUpdate");
                    InternalResponseData::ComponentPrepared
                }
                RequestData::GiveOfferExtended(_) => {
                    trace!("Got GiveOfferExtended");
                    InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_with_failure(
                        HostToken::Driver,
                        OfferRejectReason::InvalidComponent,
                        OfferStatus::Reject,
                    ))
                }
                RequestData::GiveOfferInformation(_) => {
                    trace!("Got GiveOfferInformation");
                    InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_with_failure(
                        HostToken::Driver,
                        OfferRejectReason::InvalidComponent,
                        OfferStatus::Reject,
                    ))
                }
            }
        }

        pub async fn send_response(&self, response: InternalResponseData) {
            self.cfu_device.send_response(response).await;
            trace!("Sent response: {:?}", response);
        }
    }

    impl CfuDeviceContainer for Device {
        fn get_cfu_component_device(&self) -> &CfuDevice {
            &self.cfu_device
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    pub enum UpdateState {
        /// No update has been started
        #[default]
        Idle,
        /// An update is currently in progress
        InProgress,
        /// The update has been completed
        Complete,
    }

    /// State for the struct
    #[derive(Debug, Default, Copy, Clone)]
    pub struct State {
        /// The sequence number of the final content block
        final_sequence: Option<u16>,
        /// Whether the completion status has been queried
        queried: bool,
        /// The current state of the update
        update_state: UpdateState,
    }

    /// CFU buffered component that waits for completion of the last payload
    pub struct BufferWaitComplete<'a> {
        buffer: buffer::Buffer<'a>,
        state: Mutex<GlobalRawMutex, State>,
    }

    impl<'a> BufferWaitComplete<'a> {
        pub fn new(
            external_id: ComponentId,
            buffered_id: ComponentId,
            buffer_sender: SendDynamicSender<'a, FwUpdateContentCommand>,
            buffer_receiver: SendDynamicReceiver<'a, FwUpdateContentCommand>,
            config: Config,
        ) -> Self {
            Self {
                buffer: buffer::Buffer::new(external_id, buffered_id, buffer_sender, buffer_receiver, config),
                state: Mutex::new(Default::default()),
            }
        }

        /// Wait for an event
        pub async fn wait_event(&self) -> Event {
            self.buffer.wait_event().await
        }

        /// Send a response
        pub async fn send_response(&self, response: InternalResponseData) {
            self.buffer.send_response(response).await;
        }

        /// Process a CFU request and return an optional response
        async fn process_cfu_request(&self, request: RequestData) -> Option<InternalResponseData> {
            let mut state = self.state.lock().await;
            match request {
                RequestData::GiveOfferExtended(_offer) => {
                    state.queried = true;
                    match state.update_state {
                        UpdateState::Idle => {
                            // If we are idle, reject the offer
                            Some(InternalResponseData::OfferResponse(
                                FwUpdateOfferResponse::new_with_failure(
                                    HostToken::Driver,
                                    OfferRejectReason::InvalidComponent,
                                    OfferStatus::Reject,
                                ),
                            ))
                        }
                        UpdateState::InProgress => {
                            // Don't give a respnose and block if the update is still in progress
                            None
                        }
                        UpdateState::Complete => {
                            // If we are already complete, we can immediately accept the offer
                            Some(InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_accept(
                                HostToken::Driver,
                            )))
                        }
                    }
                }
                RequestData::GiveContent(content) => {
                    if content.header.flags & FW_UPDATE_FLAG_FIRST_BLOCK != 0 {
                        // reset state
                        state.update_state = UpdateState::InProgress;
                        state.final_sequence = None;
                        state.queried = false;
                    } else if content.header.flags & FW_UPDATE_FLAG_LAST_BLOCK != 0 {
                        // If this is the last block, we can finalize the update
                        state.final_sequence = Some(content.header.sequence_num);
                    }
                    self.buffer.process(Event::CfuRequest(request)).await
                }
                _ => self.buffer.process(Event::CfuRequest(request)).await,
            }
        }

        /// Process a response from the buffered component
        async fn process_component_response(&self, response: InternalResponseData) {
            info!("Received component response");
            let mut state = self.state.lock().await;
            if let InternalResponseData::ContentResponse(response) = response {
                if Some(response.sequence) == state.final_sequence {
                    state.update_state = UpdateState::Complete;

                    // If we are blocking on completion, send the response and unblock
                    if state.queried {
                        self.buffer
                            .send_response(InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_accept(
                                HostToken::Driver,
                            )))
                            .await;
                    }
                }
            }

            // Run normal buffer logic
            self.buffer.process(Event::ComponentResponse(response)).await;
        }

        pub async fn process(&self, event: Event) -> Option<InternalResponseData> {
            match event {
                event @ Event::BufferedContent(_) => {
                    // Buffered content, don't need to do anything just pass it to the buffer
                    info!("Buffered content");
                    self.buffer.process(event).await
                }
                Event::ComponentResponse(response) => {
                    self.process_component_response(response).await;
                    None
                }
                Event::CfuRequest(request) => {
                    info!("Received CFU request");
                    self.process_cfu_request(request).await
                }
            }
        }

        pub async fn register(&'static self) -> Result<(), intrusive_list::Error> {
            self.buffer.register().await
        }
    }
}

#[embassy_executor::task]
async fn device_task(device: &'static mock::Device) {
    loop {
        let request = device.wait_request().await;
        let response = device.process_request(request).await;
        device.send_response(response).await;
    }
}

#[embassy_executor::task]
async fn buffer_task(buffer: &'static mock::BufferWaitComplete<'static>) {
    loop {
        let event = buffer.wait_event().await;
        if let Some(response) = buffer.process(event).await {
            buffer.send_response(response).await;
        }
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0: OnceLock<mock::Device> = OnceLock::new();
    let device0 = DEVICE0.get_or_init(|| {
        mock::Device::new(
            CFU_COMPONENT0_ID,
            FwVersion {
                major: 1,
                minor: 2,
                variant: 0,
            },
        )
    });
    cfu::register_device(device0).await.unwrap();
    spawner.must_spawn(device_task(device0));

    info!("Creating buffer");
    static BUFFER: OnceLock<mock::BufferWaitComplete<'static>> = OnceLock::new();
    static BUFFER_CHANNEL: OnceLock<embassy_sync::channel::Channel<GlobalRawMutex, FwUpdateContentCommand, 10>> =
        OnceLock::new();
    let channel = BUFFER_CHANNEL.get_or_init(embassy_sync::channel::Channel::new);
    let buffer = BUFFER.get_or_init(|| {
        mock::BufferWaitComplete::new(
            CFU_BUFFER_ID,
            CFU_COMPONENT0_ID,
            channel.sender().into(),
            channel.receiver().into(),
            buffer::Config::with_timeout(Duration::from_millis(75)),
        )
    });
    buffer.register().await.unwrap();
    spawner.must_spawn(buffer_task(buffer));

    info!("Getting FW version");
    let response = route_request(CFU_BUFFER_ID, RequestData::FwVersionRequest)
        .await
        .unwrap();
    let prev_version = match response {
        InternalResponseData::FwVersionResponse(response) => {
            info!("Got version response: {:#?}", response);
            Into::<u32>::into(response.component_info[0].fw_version)
        }
        _ => panic!("Unexpected response"),
    };
    info!("Got version: {:#x}", prev_version);

    info!("Giving offer");
    let offer = route_request(
        CFU_BUFFER_ID,
        RequestData::GiveOffer(FwUpdateOffer::new(
            HostToken::Driver,
            CFU_BUFFER_ID,
            FwVersion::new(0x211),
            0,
            0,
        )),
    )
    .await
    .unwrap();
    info!("Got response: {:?}", offer);

    for i in 0..10 {
        let header = FwUpdateContentHeader {
            data_length: DEFAULT_DATA_LENGTH as u8,
            sequence_num: i,
            firmware_address: 0,
            flags: if i == 0 {
                FW_UPDATE_FLAG_FIRST_BLOCK
            } else if i == 9 {
                FW_UPDATE_FLAG_LAST_BLOCK
            } else {
                0
            },
        };

        let request = FwUpdateContentCommand {
            header,
            data: [i as u8; DEFAULT_DATA_LENGTH],
        };

        info!("Giving content");
        let now = embassy_time::Instant::now();
        let response = route_request(CFU_BUFFER_ID, RequestData::GiveContent(request))
            .await
            .unwrap();
        info!("Got response in {:?} ms: {:?}", now.elapsed().as_millis(), response);
        Timer::after_millis(10).await; // Simulate some processing delay
    }

    info!("Giving special offer");
    let now = embassy_time::Instant::now();
    let response = route_request(
        CFU_BUFFER_ID,
        RequestData::GiveOfferExtended(FwUpdateOfferExtended::new(OfferExtendedComponentInfo::new(
            HostToken::Driver,
            SpecialComponentIds::Command,
            OfferCommandExtendedCodeValues::OfferNotifyOnReady,
        ))),
    )
    .await
    .unwrap();
    info!("Got response in {:?} ms: {:?}", now.elapsed().as_millis(), response);
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();
    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(cfu_service::task());
        spawner.must_spawn(run(spawner));
    });
}
