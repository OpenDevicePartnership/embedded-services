//! Integration test for UCSI

use crate::common::{DEFAULT_PER_CALL_TIMEOUT, DEFAULT_TEST_DURATION, Test, mock};

use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{DynSubscriber, WaitResult};
use embassy_time::{Timer, with_timeout};
use embedded_services::power::policy::PowerCapability;
use embedded_services::type_c::comms::UsciChangeIndicator;
use embedded_services::type_c::external::UcsiResponse;
use embedded_services::{GlobalRawMutex, type_c};
use embedded_services::{info, power};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ucsi::cci::GlobalCci;
use embedded_usb_pd::ucsi::lpm;
use embedded_usb_pd::ucsi::lpm::ResponseData as LpmResponseData;
use embedded_usb_pd::ucsi::lpm::get_connector_capability::{
    OperationModeFlags, ResponseData as UcsiConnectorCapability,
};
use embedded_usb_pd::ucsi::lpm::get_connector_status::{
    BatteryChargingCapabilityStatus, ConnectedStatus, ConnectorStatusChange,
};
use embedded_usb_pd::ucsi::ppm::ack_cc_ci::Ack;
use embedded_usb_pd::ucsi::ppm::get_capability::ResponseData as PpmCapabilities;
use embedded_usb_pd::ucsi::{Command, ResponseData as UcsiResponseData, ppm};

mod common;

const CAPABILITY: PowerCapability = PowerCapability {
    voltage_mv: 20000,
    current_ma: 5000,
};

/// Test LPM commands for a single port: connect, GetConnectorStatus, AckCcCi.
async fn test_lpm(
    port: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
    port_id: GlobalPortId,
    type_c_receiver: &mut DynSubscriber<'static, type_c::comms::CommsMessage>,
) {
    info!("Testing LPM commands for port {:?}", port_id);

    info!("Testing GetConnectorCapability");
    let expected_response = LpmResponseData::GetConnectorCapability(
        *UcsiConnectorCapability::default()
            .set_operation_mode(
                *OperationModeFlags::default()
                    .set_drp(true)
                    .set_usb2(true)
                    .set_usb3(true),
            )
            .set_consumer(true)
            .set_provider(true)
            .set_swap_to_dfp(true)
            .set_swap_to_snk(true)
            .set_swap_to_src(true),
    );
    // Don't need to push a response because the PPM overrides the LPM response.

    let response = with_timeout(
        DEFAULT_PER_CALL_TIMEOUT,
        type_c::external::execute_ucsi_command(Command::LpmCommand(lpm::GlobalCommand::new(
            port_id,
            lpm::CommandData::GetConnectorCapability,
        ))),
    )
    .await;
    assert_eq!(
        response,
        Ok(UcsiResponse {
            notify_opm: true,
            cci: *GlobalCci::default().set_cmd_complete(true),
            data: Ok(Some(UcsiResponseData::Lpm(expected_response))),
        })
    );

    // Acknowledge the CCI
    let response = with_timeout(
        DEFAULT_PER_CALL_TIMEOUT,
        type_c::external::execute_ucsi_command(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true),
        }))),
    )
    .await;
    assert_eq!(
        response,
        Ok(UcsiResponse {
            notify_opm: true,
            cci: *GlobalCci::default().set_ack_command(true),
            data: Ok(None),
        })
    );

    // Connect the port, verify UCSI event, read connector status, and acknowledge the CCI.
    port.lock().await.next_result_enable_sink_path.push_back(Ok(()));
    port.lock().await.connect_sink(CAPABILITY, false).await;

    // Give some time for the disconnect to be processed
    Timer::after(DEFAULT_PER_CALL_TIMEOUT).await;

    let message = type_c_receiver.try_next_message();
    assert_eq!(
        message,
        Some(WaitResult::Message(type_c::comms::CommsMessage::UcsiCci(
            UsciChangeIndicator {
                port: port_id,
                notify_opm: true,
            }
        )))
    );

    info!("Testing GetConnectorStatus");
    let mut status_change = ConnectorStatusChange::default();
    status_change.set_connect_change(true);
    status_change.set_battery_charging_status_change(true);

    let connected_status = ConnectedStatus {
        battery_charging_status: Some(BatteryChargingCapabilityStatus::Nominal),
        ..Default::default()
    };

    let expected_response = LpmResponseData::GetConnectorStatus(lpm::get_connector_status::ResponseData {
        status_change,
        connect_status: true,
        status: Some(connected_status),
    });

    port.lock()
        .await
        .next_result_execute_ucsi_command
        .push_back(Ok(Some(expected_response)));

    let response = with_timeout(
        DEFAULT_PER_CALL_TIMEOUT,
        type_c::external::execute_ucsi_command(Command::LpmCommand(lpm::GlobalCommand::new(
            port_id,
            lpm::CommandData::GetConnectorStatus,
        ))),
    )
    .await;

    assert_eq!(
        response,
        Ok(UcsiResponse {
            notify_opm: true,
            cci: *GlobalCci::default()
                .set_cmd_complete(true)
                // + 1 to convert between 0-based and 1-based port IDs
                .set_connector_change(GlobalPortId(port_id.0 + 1)),
            data: Ok(Some(UcsiResponseData::Lpm(expected_response))),
        })
    );

    // Acknowledge the CCI
    info!("Acknowledging CCI for port {}", port_id.0);
    let response = with_timeout(
        DEFAULT_PER_CALL_TIMEOUT,
        type_c::external::execute_ucsi_command(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true).set_connector_change(true),
        }))),
    )
    .await;
    assert_eq!(
        response,
        Ok(UcsiResponse {
            notify_opm: true,
            cci: *GlobalCci::default().set_ack_command(true),
            data: Ok(None),
        })
    );

    // Disconnect to prepare for the next test
    info!("Disconnecting port {}", port_id.0);
    port.lock().await.disconnect().await;

    // Give some time for the disconnect to be processed
    Timer::after(DEFAULT_PER_CALL_TIMEOUT).await;
    let message = type_c_receiver.try_next_message();
    assert_eq!(
        message,
        Some(WaitResult::Message(type_c::comms::CommsMessage::UcsiCci(
            UsciChangeIndicator {
                port: port_id,
                notify_opm: true,
            }
        )))
    );

    // Get disconnected port status
    info!("Getting disconnected port status for port {}", port_id.0);
    let expected_response = LpmResponseData::GetConnectorStatus(lpm::get_connector_status::ResponseData::default());
    port.lock()
        .await
        .next_result_execute_ucsi_command
        .push_back(Ok(Some(expected_response)));

    let response = with_timeout(
        DEFAULT_PER_CALL_TIMEOUT,
        type_c::external::execute_ucsi_command(Command::LpmCommand(lpm::GlobalCommand::new(
            port_id,
            lpm::CommandData::GetConnectorStatus,
        ))),
    )
    .await;

    assert_eq!(
        response,
        Ok(UcsiResponse {
            notify_opm: true,
            cci: *GlobalCci::default()
                .set_cmd_complete(true)
                // + 1 to convert between 0-based and 1-based port IDs
                .set_connector_change(GlobalPortId(port_id.0 + 1)),
            data: Ok(Some(UcsiResponseData::Lpm(expected_response))),
        })
    );

    info!("Acknowledging CCI for port {}", port_id.0);
    let response = with_timeout(
        DEFAULT_PER_CALL_TIMEOUT,
        type_c::external::execute_ucsi_command(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true).set_connector_change(true),
        }))),
    )
    .await;
    assert_eq!(
        response,
        Ok(UcsiResponse {
            notify_opm: true,
            cci: *GlobalCci::default().set_ack_command(true),
            data: Ok(None),
        })
    );
}

struct TestUcsi;

impl Test for TestUcsi {
    async fn run(
        &mut self,
        mut type_c_receiver: DynSubscriber<'static, type_c::comms::CommsMessage>,
        _power_policy_event_receiver: DynSubscriber<'static, power::policy::CommsMessage>,
        port0: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        port1: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        port2: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
    ) {
        // Reset the PPM
        info!("PPM Reset");
        let response = with_timeout(
            DEFAULT_PER_CALL_TIMEOUT,
            type_c::external::execute_ucsi_command(Command::PpmCommand(ppm::Command::PpmReset)),
        )
        .await;
        assert_eq!(
            response,
            Ok(UcsiResponse {
                // OPM is supposed to poll for the reset complete flag
                notify_opm: false,
                cci: *GlobalCci::default().set_reset_complete(true),
                data: Ok(None),
            })
        );

        // Enable notifications
        info!("Enabling notifications");
        let mut notifications = embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable::default();
        notifications.set_cmd_complete(true);
        notifications.set_connect_change(true);
        let response = with_timeout(
            DEFAULT_PER_CALL_TIMEOUT,
            type_c::external::execute_ucsi_command(Command::PpmCommand(ppm::Command::SetNotificationEnable(
                ppm::set_notification_enable::Args {
                    notification_enable: notifications,
                },
            ))),
        )
        .await;
        assert_eq!(
            response,
            Ok(UcsiResponse {
                notify_opm: true,
                cci: *GlobalCci::default().set_cmd_complete(true),
                data: Ok(None),
            })
        );

        let response = with_timeout(
            DEFAULT_PER_CALL_TIMEOUT,
            type_c::external::execute_ucsi_command(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
                ack: *Ack::default().set_command_complete(true),
            }))),
        )
        .await;
        assert_eq!(
            response,
            Ok(UcsiResponse {
                notify_opm: true,
                cci: *GlobalCci::default().set_ack_command(true),
                data: Ok(None),
            })
        );

        test_lpm(port0, GlobalPortId(0), &mut type_c_receiver).await;
        test_lpm(port1, GlobalPortId(1), &mut type_c_receiver).await;
        test_lpm(port2, GlobalPortId(2), &mut type_c_receiver).await;
    }
}

#[tokio::test]
async fn ucsi() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        type_c_service::service::config::Config {
            ucsi_capabilities: PpmCapabilities {
                num_connectors: 3,
                bcd_usb_pd_spec: 0x0300,
                bcd_type_c_spec: 0x0200,
                bcd_battery_charging_spec: 0x0120,
                ..Default::default()
            },
            ucsi_port_capabilities: Some(
                *UcsiConnectorCapability::default()
                    .set_operation_mode(
                        *OperationModeFlags::default()
                            .set_drp(true)
                            .set_usb2(true)
                            .set_usb3(true),
                    )
                    .set_consumer(true)
                    .set_provider(true)
                    .set_swap_to_dfp(true)
                    .set_swap_to_snk(true)
                    .set_swap_to_src(true),
            ),
            ..Default::default()
        },
        power_policy_service::config::Config::default(),
        TestUcsi,
    )
    .await;
}
