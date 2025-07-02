use embassy_executor::{Executor, Spawner};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, once_lock::OnceLock};
use embassy_time::{Duration, Timer};
use log::*;
use static_cell::StaticCell;

use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::cfu::{self, component::InternalResponseData, route_request};

use cfu_service::buffer;

use crate::cfu::component::RequestData;

/// Component ID for the CFU buffer
const CFU_BUFFER_ID: ComponentId = 0x06;

/// Component ID for the mock device
const CFU_COMPONENT0_ID: ComponentId = 0x20;

mod mock {
    use std::cell::Cell;

    use embedded_services::cfu::component::{CfuDevice, CfuDeviceContainer, InternalResponseData};

    use super::*;

    /// Mock CFU device
    pub struct Device {
        cfu_device: CfuDevice,
        version: FwVersion,
        init: Cell<bool>,
    }

    impl Device {
        /// Create a new mock CFU device
        pub fn new(component_id: ComponentId, version: FwVersion) -> Self {
            Self {
                cfu_device: CfuDevice::new(component_id),
                version,
                init: Cell::new(false),
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
                    if self.init.get() {
                        trace!("Got GiveContent: {content:#?}");
                        // If the device is already initialized, accept the content
                        InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                            content.header.sequence_num,
                            CfuUpdateContentResponseStatus::Success,
                        ))
                    } else {
                        // Take 500 ms to init the device
                        self.init.set(true);
                        info!("Initializing device, taking 500 ms");
                        embassy_time::Timer::after_millis(500).await;
                        info!("Device initialized");
                        InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                            content.header.sequence_num,
                            CfuUpdateContentResponseStatus::Success,
                        ))
                    }
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

        pub async fn send_response(&self, response: InternalResponseData) {
            self.cfu_device.send_response(response).await;
            trace!("Sent response: {response:?}");
        }
    }

    impl CfuDeviceContainer for Device {
        fn get_cfu_component_device(&self) -> &CfuDevice {
            &self.cfu_device
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
async fn buffer_task(buffer: &'static buffer::Buffer<'static>) {
    loop {
        let request = buffer.wait_event().await;
        if let Some(response) = buffer.process(request).await {
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
    static BUFFER: OnceLock<buffer::Buffer<'static>> = OnceLock::new();
    static BUFFER_CHANNEL: OnceLock<embassy_sync::channel::Channel<NoopRawMutex, FwUpdateContentCommand, 10>> =
        OnceLock::new();
    let channel = BUFFER_CHANNEL.get_or_init(embassy_sync::channel::Channel::new);
    let buffer = BUFFER.get_or_init(|| {
        buffer::Buffer::new(
            CFU_BUFFER_ID,
            CFU_COMPONENT0_ID,
            channel.dyn_sender(),
            channel.dyn_receiver(),
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
            info!("Got version response: {response:#?}");
            Into::<u32>::into(response.component_info[0].fw_version)
        }
        _ => panic!("Unexpected response"),
    };
    info!("Got version: {prev_version:#x}");

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
    info!("Got response: {offer:?}");

    for i in 0..10 {
        let header = FwUpdateContentHeader {
            data_length: DEFAULT_DATA_LENGTH as u8,
            sequence_num: i,
            firmware_address: 0,
            flags: if i == 0 { FW_UPDATE_FLAG_FIRST_BLOCK } else { 0 },
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

    info!("data: {}", size_of::<RequestData>());
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
