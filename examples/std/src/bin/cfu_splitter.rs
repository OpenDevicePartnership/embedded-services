use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use log::*;
use static_cell::StaticCell;

use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::cfu::{self, component::InternalResponseData, route_request};

use cfu_service::splitter;

use crate::cfu::component::RequestData;

/// Component ID for the CFU Splitter
const CFU_SPLITTER_ID: ComponentId = 0x06;

/// Component ID for the first mock device
const CFU_COMPONENT0_ID: ComponentId = 0x20;
/// Component ID for the second mock device
const CFU_COMPONENT1_ID: ComponentId = 0x21;

mod mock {
    use embedded_services::cfu::component::{CfuDevice, CfuDeviceContainer, InternalResponseData};

    use super::*;

    /// Struct to contain the customization logic for the mock Splitter
    pub struct Customization {}

    impl splitter::Customization for Customization {
        fn resolve_fw_versions(&self, versions: &[GetFwVersionResponse]) -> GetFwVersionResponse {
            for version in versions {
                info!("Supplied FW version: {:?}", version);
            }
            // For simplicity, just return the first version
            versions[0]
        }

        fn resolve_offer_response(&self, offer_responses: &[FwUpdateOfferResponse]) -> FwUpdateOfferResponse {
            for offer in offer_responses {
                info!("Supplied Offer Response: {:?}", offer);
                if offer.status == OfferStatus::Reject {
                    // Exit on the first rejection
                    error!("Offer rejected: {:?}", offer.reject_reason);
                    return *offer;
                }
            }

            // Otherwise, return the first accepted offer
            offer_responses[0]
        }

        fn resolve_content_response(&self, content_responses: &[FwUpdateContentResponse]) -> FwUpdateContentResponse {
            for content in content_responses {
                info!("Supplied Content Response: {:?}", content);
                if content.status != CfuUpdateContentResponseStatus::Success {
                    // Exit on the first failure
                    error!("Content response failed: {:?}", content.status);
                    return *content;
                }
            }
            content_responses[0]
        }
    }

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
                    trace!("Got GiveContent");
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
            }
        }

        pub async fn send_response(&self, response: InternalResponseData) {
            self.cfu_device.send_response(response).await;
        }
    }

    impl CfuDeviceContainer for Device {
        fn get_cfu_component_device(&self) -> &CfuDevice {
            &self.cfu_device
        }
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn device_task(device: &'static mock::Device) {
    loop {
        let request = device.wait_request().await;
        let response = device.process_request(request).await;
        device.send_response(response).await;
    }
}

#[embassy_executor::task]
async fn splitter_task(splitter: &'static splitter::Splitter<'static, mock::Customization>) {
    loop {
        let request = splitter.wait_request().await;
        let response = splitter.process_request(request).await;
        splitter.send_response(response).await;
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

    info!("Creating device 1");
    static DEVICE1: OnceLock<mock::Device> = OnceLock::new();
    let device1 = DEVICE1.get_or_init(|| {
        mock::Device::new(
            CFU_COMPONENT1_ID,
            FwVersion {
                major: 2,
                minor: 1,
                variant: 3,
            },
        )
    });
    cfu::register_device(device1).await.unwrap();
    spawner.must_spawn(device_task(device1));

    info!("Creating splitter");
    static SPLITTER: OnceLock<splitter::Splitter<'static, mock::Customization>> = OnceLock::new();
    static DEVICES: [ComponentId; 2] = [CFU_COMPONENT0_ID, CFU_COMPONENT1_ID];
    let customization = mock::Customization {};
    let splitter = SPLITTER.get_or_init(|| splitter::Splitter::new(CFU_SPLITTER_ID, &DEVICES, customization).unwrap());
    splitter.register().await.unwrap();
    spawner.must_spawn(splitter_task(splitter));

    info!("Getting FW version");
    let response = route_request(CFU_SPLITTER_ID, RequestData::FwVersionRequest)
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
        CFU_SPLITTER_ID,
        RequestData::GiveOffer(FwUpdateOffer::new(
            HostToken::Driver,
            CFU_SPLITTER_ID,
            FwVersion::new(0x211),
            0,
            0,
        )),
    )
    .await
    .unwrap();
    info!("Got response: {:?}", offer);

    let header = FwUpdateContentHeader {
        data_length: DEFAULT_DATA_LENGTH as u8,
        sequence_num: 0,
        firmware_address: 0,
        flags: FW_UPDATE_FLAG_FIRST_BLOCK,
    };

    let request = FwUpdateContentCommand {
        header,
        data: [0u8; DEFAULT_DATA_LENGTH],
    };

    let response = route_request(CFU_SPLITTER_ID, RequestData::GiveContent(request))
        .await
        .unwrap();
    info!("Got response: {:?}", response);
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(cfu_service::task());
        spawner.must_spawn(run(spawner));
    });
}
