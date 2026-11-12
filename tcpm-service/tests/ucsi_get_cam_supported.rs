//! Integration tests for GET_CAM_SUPPORTED response lengths.
#![allow(dead_code)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use embassy_time::with_timeout;
use embedded_usb_pd::ucsi::v1_2::cci::GlobalCci;
use embedded_usb_pd::ucsi::v1_2::{GlobalCommand, ResponseData, lpm, ppm};
use embedded_usb_pd::vdm::{AltModeId, structured::Svid};
use embedded_usb_pd::{GlobalPortId, LocalPortId, PdError};
use tcpm_interface_test_mocks::controller::{FnCall, ucsi::FnCall as UcsiFnCall};

use common::{
    DEFAULT_PER_CALL_TIMEOUT, DEFAULT_TEST_DURATION, PowerPolicyServiceReceiver, Test, TestPort, TypeCServiceMutexType,
    TypeCServiceReceiver,
};

mod common;

async fn execute_and_assert(
    service: &TypeCServiceMutexType<'_, '_>,
    command: GlobalCommand,
    expected_notify_opm: bool,
    expected_cci: GlobalCci,
    expected_data: Result<Option<ResponseData>, PdError>,
) {
    let response = with_timeout(DEFAULT_PER_CALL_TIMEOUT, async {
        service.lock().await.process_ucsi_command(&command).await
    })
    .await
    .expect("UCSI command timed out");
    assert_eq!(response.notify_opm, expected_notify_opm);
    assert_eq!(response.cci, expected_cci, "unexpected CCI");
    assert_eq!(response.data, expected_data);
}

async fn acknowledge_command(service: &TypeCServiceMutexType<'_, '_>) {
    execute_and_assert(
        service,
        GlobalCommand::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: ppm::ack_cc_ci::Ack {
                command_complete: true,
                ..Default::default()
            },
        })),
        true,
        GlobalCci {
            ack_command: true,
            ..Default::default()
        },
        Ok(None),
    )
    .await;
}

struct TestGetCamSupported {
    alt_modes_count: usize,
    expected_data_len: u8,
}

impl Test for TestGetCamSupported {
    async fn run<'port, 'ch>(
        &mut self,
        service: &TypeCServiceMutexType<'port, 'ch>,
        _type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        _power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        execute_and_assert(
            service,
            GlobalCommand::PpmCommand(ppm::Command::PpmReset),
            false,
            GlobalCci {
                reset_complete: true,
                ..Default::default()
            },
            Ok(None),
        )
        .await;

        execute_and_assert(
            service,
            GlobalCommand::PpmCommand(ppm::Command::SetNotificationEnable(
                ppm::set_notification_enable::Args {
                    notification_enable: ppm::set_notification_enable::NotificationEnable {
                        cmd_complete: true,
                        ..Default::default()
                    },
                },
            )),
            true,
            GlobalCci {
                cmd_complete: true,
                ..Default::default()
            },
            Ok(None),
        )
        .await;
        acknowledge_command(service).await;

        let mut expected_commands = Vec::new();
        let mut cam_supported = lpm::get_cam_supported::ResponseData::default();
        for index in 0..self.alt_modes_count {
            cam_supported.try_set_supported(index, index % 2 == 0).unwrap();
        }
        let expected_data = lpm::ResponseData::GetCamSupported(cam_supported);
        {
            let mut mock = port0.mock.lock().await;
            // A full final page requires another request to discover the end of the list.
            for offset in (0..=self.alt_modes_count).step_by(lpm::get_alternate_modes::MAX_ALT_MODES) {
                let mut page = lpm::get_alternate_modes::ResponseData::default();
                for _ in 0..(self.alt_modes_count - offset).min(lpm::get_alternate_modes::MAX_ALT_MODES) {
                    page.alt_modes
                        .push(lpm::get_alternate_modes::AltMode {
                            svid: Svid(0xff01),
                            mid: AltModeId(1),
                        })
                        .unwrap();
                }
                mock.next_result_execute_lpm_command
                    .push_back(Ok(Some(lpm::ResponseData::GetAlternateModes(page))));
                expected_commands.push(lpm::LocalCommand::new(lpm::CommandData::GetAlternateModes(
                    lpm::get_alternate_modes::Args {
                        connector_number: LocalPortId(0),
                        recipient: lpm::Recipient::Connector,
                        mode_offset: offset as u8,
                        num_modes: lpm::get_alternate_modes::MAX_ALT_MODES as u8,
                    },
                )));
            }
            mock.next_result_execute_lpm_command
                .push_back(Ok(Some(expected_data.clone())));
        }
        expected_commands.push(lpm::LocalCommand::new(lpm::CommandData::GetCamSupported(
            lpm::get_cam_supported::Args {
                connector_number: LocalPortId(0),
            },
        )));

        execute_and_assert(
            service,
            GlobalCommand::LpmCommand(lpm::GlobalCommand::new(lpm::CommandData::GetCamSupported(
                lpm::get_cam_supported::Args {
                    connector_number: GlobalPortId(0),
                },
            ))),
            true,
            GlobalCci {
                cmd_complete: true,
                data_len: self.expected_data_len,
                ..Default::default()
            },
            Ok(Some(ResponseData::Lpm(expected_data))),
        )
        .await;

        {
            let mut mock = port0.mock.lock().await;
            for expected_command in expected_commands {
                let Some(FnCall::Ucsi(UcsiFnCall::ExecuteLpm(command))) = mock.fn_calls.pop_front() else {
                    panic!("expected an LPM command");
                };
                assert_eq!(command, expected_command);
            }
            assert!(mock.fn_calls.is_empty());
            assert!(mock.next_result_execute_lpm_command.is_empty());
        }
        acknowledge_command(service).await;
    }
}

async fn run_get_cam_supported_test(alt_modes_count: usize, expected_data_len: u8) {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestGetCamSupported {
            alt_modes_count,
            expected_data_len,
        },
    )
    .await;
}

#[tokio::test]
async fn no_alternate_modes() {
    run_get_cam_supported_test(0, 0).await;
}

#[tokio::test]
async fn single_alternate_mode() {
    run_get_cam_supported_test(1, 1).await;
}

#[tokio::test]
async fn full_alternate_modes_page() {
    run_get_cam_supported_test(2, 1).await;
}

#[tokio::test]
async fn eight_alternate_modes() {
    run_get_cam_supported_test(8, 1).await;
}

#[tokio::test]
async fn nine_alternate_modes() {
    run_get_cam_supported_test(9, 2).await;
}

#[tokio::test]
async fn sixteen_alternate_modes() {
    run_get_cam_supported_test(16, 2).await;
}

#[tokio::test]
async fn seventeen_alternate_modes() {
    run_get_cam_supported_test(17, 3).await;
}
