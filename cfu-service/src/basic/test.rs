//! Tests for [`crate::basic::Updater`]
#![allow(clippy::unwrap_used)]
extern crate std;

use crate::{
    basic::{
        Output, Updater,
        event_receiver::Event,
        state::{FwUpdateState, SharedState},
    },
    component::{InternalResponseData, RequestData},
};
use embassy_sync::{mutex::Mutex, once_lock::OnceLock};
use embassy_time::{Duration, with_timeout};
use embedded_cfu_protocol::protocol_definitions::{
    CfuUpdateContentResponseStatus, DEFAULT_DATA_LENGTH, FW_UPDATE_FLAG_FIRST_BLOCK, FW_UPDATE_FLAG_LAST_BLOCK,
    FwUpdateContentCommand, FwUpdateContentHeader, FwUpdateContentResponse, FwUpdateOffer, FwUpdateOfferResponse,
    FwVerComponentInfo, FwVersion, GetFwVerRespHeaderByte3, GetFwVersionResponse, GetFwVersionResponseHeader,
    HostToken, MAX_CMPT_COUNT,
};
use embedded_services::GlobalRawMutex;

use crate::mocks::customization::{FnCall as CustomizationFnCall, Mock as MockCustomization};
use fw_update_interface_mocks::basic::{FnCall as FwFnCall, Mock};

use std::vec;

const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

const CURRENT_FW_VERSION: u32 = 0x12345678;
const NEW_FW_VERSION: u32 = 0x89abcdef;

const DEVICE0_COMPONENT_ID: u8 = 5;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

type DeviceType = Mutex<GlobalRawMutex, Mock>;
type SharedStateType = Mutex<GlobalRawMutex, SharedState>;
type UpdaterType<'a> = Updater<'a, DeviceType, SharedStateType, MockCustomization>;

/// Test the basic flow of the updater.
///
/// This will get the FW version, give an offer, then send a start, middle, and end content
pub struct TestBasicFlow;

impl Test for TestBasicFlow {
    async fn run<'a>(&mut self, device: &'a DeviceType, cfu_basic: &'a mut UpdaterType<'a>) {
        {
            // Get FW version
            let output = with_timeout(
                PER_CALL_TIMEOUT,
                cfu_basic.process_event(Event::Request(RequestData::FwVersionRequest)),
            )
            .await
            .unwrap();

            assert_eq!(
                output,
                Output::CfuResponse(InternalResponseData::FwVersionResponse(GetFwVersionResponse {
                    header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
                    component_info: [FwVerComponentInfo::new(FwVersion::new(CURRENT_FW_VERSION), DEVICE0_COMPONENT_ID);
                        MAX_CMPT_COUNT],
                }))
            );
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::Idle);
            assert_eq!(device.lock().await.fn_calls.len(), 1);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::GetActiveFwVersion
            );
        }

        {
            // Give offer
            let output = with_timeout(
                PER_CALL_TIMEOUT,
                cfu_basic.process_event(Event::Request(RequestData::GiveOffer(FwUpdateOffer::new(
                    HostToken::Driver,
                    DEVICE0_COMPONENT_ID,
                    FwVersion::new(NEW_FW_VERSION),
                    0,
                    0,
                )))),
            )
            .await
            .unwrap();

            assert_eq!(
                output,
                Output::CfuResponse(InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_accept(
                    HostToken::Driver
                )))
            );
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::Idle);
            assert_eq!(device.lock().await.fn_calls.len(), 1);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::GetActiveFwVersion
            );

            assert_eq!(cfu_basic.customization().fn_calls.len(), 1);
            assert_eq!(
                cfu_basic.customization_mut().fn_calls.pop_front(),
                Some(CustomizationFnCall::Validate(
                    FwVersion::new(CURRENT_FW_VERSION),
                    FwUpdateOffer::new(
                        HostToken::Driver,
                        DEVICE0_COMPONENT_ID,
                        FwVersion::new(NEW_FW_VERSION),
                        0,
                        0,
                    )
                ))
            );
        }

        {
            // Give first content block
            let output = with_timeout(
                PER_CALL_TIMEOUT,
                cfu_basic.process_event(Event::Request(RequestData::GiveContent(FwUpdateContentCommand {
                    header: FwUpdateContentHeader {
                        flags: FW_UPDATE_FLAG_FIRST_BLOCK,
                        data_length: DEFAULT_DATA_LENGTH as u8,
                        sequence_num: 0,
                        firmware_address: 0x0,
                    },
                    data: [1; DEFAULT_DATA_LENGTH],
                }))),
            )
            .await
            .unwrap();

            assert_eq!(
                output,
                Output::CfuResponse(InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    0,
                    CfuUpdateContentResponseStatus::Success
                )))
            );
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::InProgress(0));
            assert_eq!(device.lock().await.fn_calls.len(), 2);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::StartFwUpdate
            );
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::WriteFwContents(0, vec![1; DEFAULT_DATA_LENGTH])
            );
        }

        {
            // Give middle content block
            let output = with_timeout(
                PER_CALL_TIMEOUT,
                cfu_basic.process_event(Event::Request(RequestData::GiveContent(FwUpdateContentCommand {
                    header: FwUpdateContentHeader {
                        flags: 0,
                        data_length: DEFAULT_DATA_LENGTH as u8,
                        sequence_num: 1,
                        firmware_address: 0x0,
                    },
                    data: [2; DEFAULT_DATA_LENGTH],
                }))),
            )
            .await
            .unwrap();

            assert_eq!(
                output,
                Output::CfuResponse(InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    1,
                    CfuUpdateContentResponseStatus::Success
                )))
            );
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::InProgress(0));
            assert_eq!(device.lock().await.fn_calls.len(), 1);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::WriteFwContents(0, vec![2; DEFAULT_DATA_LENGTH])
            );
        }

        {
            // Give final content block
            let output = with_timeout(
                PER_CALL_TIMEOUT,
                cfu_basic.process_event(Event::Request(RequestData::GiveContent(FwUpdateContentCommand {
                    header: FwUpdateContentHeader {
                        flags: FW_UPDATE_FLAG_LAST_BLOCK,
                        data_length: DEFAULT_DATA_LENGTH as u8,
                        sequence_num: 2,
                        firmware_address: 0x0,
                    },
                    data: [3; DEFAULT_DATA_LENGTH],
                }))),
            )
            .await
            .unwrap();

            assert_eq!(
                output,
                Output::CfuResponse(InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    2,
                    CfuUpdateContentResponseStatus::Success
                )))
            );
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::Idle);
            assert_eq!(device.lock().await.fn_calls.len(), 2);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::WriteFwContents(0, vec![3; DEFAULT_DATA_LENGTH])
            );
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::FinalizeFwUpdate
            );
        }
    }
}

/// Test that the recovery flow works immediately after sending the first content block.
struct TestStartRecoveryFlow;

impl Test for TestStartRecoveryFlow {
    async fn run<'a>(&mut self, device: &'a DeviceType, cfu_basic: &'a mut UpdaterType<'a>) {
        {
            // Give offer
            let output = with_timeout(
                PER_CALL_TIMEOUT,
                cfu_basic.process_event(Event::Request(RequestData::GiveOffer(FwUpdateOffer::new(
                    HostToken::Driver,
                    DEVICE0_COMPONENT_ID,
                    FwVersion::new(NEW_FW_VERSION),
                    0,
                    0,
                )))),
            )
            .await
            .unwrap();

            assert_eq!(
                output,
                Output::CfuResponse(InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_accept(
                    HostToken::Driver
                )))
            );
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::Idle);
            assert_eq!(device.lock().await.fn_calls.len(), 1);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::GetActiveFwVersion
            );

            assert_eq!(cfu_basic.customization().fn_calls.len(), 1);
            assert_eq!(
                cfu_basic.customization_mut().fn_calls.pop_front(),
                Some(CustomizationFnCall::Validate(
                    FwVersion::new(CURRENT_FW_VERSION),
                    FwUpdateOffer::new(
                        HostToken::Driver,
                        DEVICE0_COMPONENT_ID,
                        FwVersion::new(NEW_FW_VERSION),
                        0,
                        0,
                    )
                ))
            );
        }

        {
            // Give first content block
            let output = with_timeout(
                PER_CALL_TIMEOUT,
                cfu_basic.process_event(Event::Request(RequestData::GiveContent(FwUpdateContentCommand {
                    header: FwUpdateContentHeader {
                        flags: FW_UPDATE_FLAG_FIRST_BLOCK,
                        data_length: DEFAULT_DATA_LENGTH as u8,
                        sequence_num: 0,
                        firmware_address: 0x0,
                    },
                    data: [1; DEFAULT_DATA_LENGTH],
                }))),
            )
            .await
            .unwrap();

            assert_eq!(
                output,
                Output::CfuResponse(InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    0,
                    CfuUpdateContentResponseStatus::Success
                )))
            );
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::InProgress(0));
            assert_eq!(device.lock().await.fn_calls.len(), 2);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::StartFwUpdate
            );
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::WriteFwContents(0, vec![1; DEFAULT_DATA_LENGTH])
            );
        }

        {
            // Trigger recovery
            let output = with_timeout(PER_CALL_TIMEOUT, cfu_basic.process_event(Event::RecoveryTick))
                .await
                .unwrap();

            assert_eq!(output, Output::CfuRecovery);
            // Idle since we should have successfully recovered
            assert_eq!(cfu_basic.update_state().await, FwUpdateState::Idle);
            assert_eq!(device.lock().await.fn_calls.len(), 1);
            assert_eq!(
                device.lock().await.fn_calls.pop_front().unwrap(),
                FwFnCall::AbortFwUpdate
            );
        }
    }
}

#[tokio::test]
async fn run_test_basic_flow() {
    run_test(DEFAULT_TIMEOUT, TestBasicFlow).await;
}

#[tokio::test]
async fn run_test_start_recovery_flow() {
    run_test(DEFAULT_TIMEOUT, TestStartRecoveryFlow).await;
}

/// Trait for runnable tests.
///
/// This exists because there are lifetime issues with being generic over FnOnce or FnMut.
/// Those can be resolved, but having a dedicated trait is simpler.
pub trait Test {
    fn run<'a>(&mut self, device: &'a DeviceType, cfu_basic: &'a mut UpdaterType<'a>) -> impl Future<Output = ()>;
}

/// Test running function
async fn run_test(timeout: Duration, mut test: impl Test) {
    // Tokio runs tests in parallel, but logging is global so we need to run tests sequentially to avoid interleaved logs.
    static TEST_MUTEX: OnceLock<Mutex<GlobalRawMutex, ()>> = OnceLock::new();
    let test_mutex = TEST_MUTEX.get_or_init(|| Mutex::new(()));
    let _lock = test_mutex.lock().await;

    // Initialize logging, ignore the error if the logger was already initialized by another test.
    let _ = env_logger::builder().filter_level(log::LevelFilter::Debug).try_init();
    embedded_services::init().await;

    let shared_state: Mutex<GlobalRawMutex, _> = Mutex::new(SharedState::default());
    let device = Mutex::new(Mock::new("PSU0", CURRENT_FW_VERSION));
    let mut cfu_basic = Updater::new(
        &device,
        &shared_state,
        Default::default(),
        DEVICE0_COMPONENT_ID,
        MockCustomization::new(FwVersion::new(NEW_FW_VERSION)),
    );

    with_timeout(timeout, test.run(&device, &mut cfu_basic)).await.unwrap();
}
