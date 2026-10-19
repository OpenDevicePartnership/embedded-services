//! Integration test for UCSI
#![allow(dead_code)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use embassy_time::with_timeout;
use embedded_usb_pd::type_c::ConnectionState;
use embedded_usb_pd::ucsi::v1_2::cci::GlobalCci;
use embedded_usb_pd::ucsi::v1_2::lpm::get_connector_capability::{
    OperationModeFlags, ResponseData as UcsiConnectorCapability,
};
use embedded_usb_pd::ucsi::v1_2::lpm::get_connector_status::{
    BatteryChargingCapabilityStatus, ConnectedStatus, ConnectorStatusChange,
};
use embedded_usb_pd::ucsi::v1_2::lpm::{self, ResponseData as LpmResponseData};
use embedded_usb_pd::ucsi::v1_2::ppm::{
    self, ack_cc_ci::Ack, get_capability::ResponseData as PpmCapabilities, set_notification_enable::NotificationEnable,
};
use embedded_usb_pd::ucsi::v1_2::{GlobalCommand, ResponseData as UcsiResponseData};
use embedded_usb_pd::{GlobalPortId, PdError, PowerRole};
use log::info;
use tcpm_interface::control::pd::PortStatus;
use tcpm_interface::port::event::{PortEvent, PortStatusEventBitfield};
use tcpm_interface::service::event::EventData;
use tcpm_service::controller::event::Event;

use crate::common::{
    DEFAULT_PER_CALL_TIMEOUT, DEFAULT_TEST_DURATION, PowerPolicyServiceReceiver, Test, TestPort, TypeCServiceMutexType,
    TypeCServiceReceiver,
};

mod common;

/// Execute a UCSI command against the service and assert the resulting response fields.
///
/// The response type does not implement `PartialEq`, so fields are compared individually rather than as a whole.
async fn execute_and_assert(
    service: &TypeCServiceMutexType<'_, '_>,
    command: GlobalCommand,
    expected_notify_opm: bool,
    expected_cci: GlobalCci,
    expected_data: Result<Option<UcsiResponseData>, PdError>,
) {
    let response = with_timeout(DEFAULT_PER_CALL_TIMEOUT, async {
        service.lock().await.process_ucsi_command(&command).await
    })
    .await
    .expect("UCSI command timed out");

    assert_eq!(response.notify_opm, expected_notify_opm, "unexpected notify_opm");
    assert_eq!(response.cci, expected_cci, "unexpected CCI");
    assert_eq!(response.data, expected_data, "unexpected response data");
}

/// Simulate a plug event that attaches a sink so the service raises a UCSI connector change.
async fn connect_port(port: &TestPort<'_, '_>) {
    port.mock
        .lock()
        .await
        .next_result_get_port_status
        .push_back(Ok(PortStatus {
            connection_state: Some(ConnectionState::Attached),
            power_role: PowerRole::Sink,
            ..Default::default()
        }));

    let mut status_event = PortStatusEventBitfield::none();
    status_event.set_plug_inserted_or_removed(true);
    port.port
        .lock()
        .await
        .process_event(Event::PortEvent(PortEvent::StatusChanged(status_event)))
        .await
        .unwrap();
}

/// Simulate an unplug event so the service raises a UCSI connector change.
async fn disconnect_port(port: &TestPort<'_, '_>) {
    port.mock
        .lock()
        .await
        .next_result_get_port_status
        .push_back(Ok(PortStatus::default()));

    let mut status_event = PortStatusEventBitfield::none();
    status_event.set_plug_inserted_or_removed(true);
    port.port
        .lock()
        .await
        .process_event(Event::PortEvent(PortEvent::StatusChanged(status_event)))
        .await
        .unwrap();
}

/// Receive a single UCSI connector-change notification and assert it targets `port_id`.
async fn expect_ucsi_cci(receiver: &TypeCServiceReceiver<'_, '_>, port_id: GlobalPortId) {
    let event = with_timeout(DEFAULT_PER_CALL_TIMEOUT, receiver.receive())
        .await
        .expect("did not receive UCSI connector change notification");
    match event.event {
        EventData::UsciChangeIndicator(data) => {
            assert_eq!(data.port, port_id);
            assert!(data.notify_opm, "expected notify_opm to be set");
        }
        other => panic!("expected UsciChangeIndicator, got {other:?}"),
    }
}

/// Test LPM commands for a single port: GetConnectorCapability, connect, GetConnectorStatus, AckCcCi.
async fn test_lpm(
    service: &TypeCServiceMutexType<'_, '_>,
    type_c_receiver: &TypeCServiceReceiver<'_, '_>,
    port: &TestPort<'_, '_>,
    port_id: GlobalPortId,
) {
    info!("Testing LPM commands for port {port_id:?}");

    // GetConnectorCapability is served from the config override, so no mock response is needed.
    let expected_capability = LpmResponseData::GetConnectorCapability(port_capability());
    execute_and_assert(
        service,
        GlobalCommand::LpmCommand(lpm::GlobalCommand::new(lpm::CommandData::GetConnectorCapability(
            lpm::get_connector_capability::Args {
                connector_number: port_id,
            },
        ))),
        true,
        GlobalCci {
            cmd_complete: true,
            ..Default::default()
        },
        Ok(Some(UcsiResponseData::Lpm(expected_capability))),
    )
    .await;

    // Acknowledge the CCI.
    execute_and_assert(
        service,
        GlobalCommand::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: Ack {
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

    // Connect the port and verify the UCSI connector-change notification.
    info!("Connecting port {}", port_id.0);
    connect_port(port).await;
    expect_ucsi_cci(type_c_receiver, port_id).await;

    // GetConnectorStatus while connected.
    info!("Testing GetConnectorStatus for connected port {}", port_id.0);
    let status_change = ConnectorStatusChange {
        connect_change: true,
        battery_charging_status_change: true,
        ..Default::default()
    };
    let connected_status = ConnectedStatus {
        battery_charging_status: Some(BatteryChargingCapabilityStatus::Nominal),
        ..Default::default()
    };
    let connected_response = LpmResponseData::GetConnectorStatus(lpm::get_connector_status::ResponseData {
        status_change,
        connect_status: true,
        status: Some(connected_status),
    });
    {
        let mut mock = port.mock.lock().await;
        mock.next_result_execute_lpm_command
            .push_back(Ok(Some(connected_response)));
        // Filling in the battery charging capability re-reads the port status.
        mock.next_result_get_port_status.push_back(Ok(PortStatus {
            connection_state: Some(ConnectionState::Attached),
            power_role: PowerRole::Sink,
            ..Default::default()
        }));
    }
    execute_and_assert(
        service,
        get_connector_status_command(port_id),
        true,
        GlobalCci {
            cmd_complete: true,
            // UCSI connector numbers are 1-based.
            connector_change: GlobalPortId(port_id.0 + 1),
            ..Default::default()
        },
        Ok(Some(UcsiResponseData::Lpm(connected_response))),
    )
    .await;

    // Acknowledge the connector change.
    info!("Acknowledging CCI for port {}", port_id.0);
    ack_command_and_connector_change(service).await;

    // Disconnect and verify the UCSI connector-change notification.
    info!("Disconnecting port {}", port_id.0);
    disconnect_port(port).await;
    expect_ucsi_cci(type_c_receiver, port_id).await;

    // GetConnectorStatus while disconnected.
    info!("Getting disconnected port status for port {}", port_id.0);
    let disconnected_response = LpmResponseData::GetConnectorStatus(lpm::get_connector_status::ResponseData {
        connect_status: false,
        ..Default::default()
    });
    port.mock
        .lock()
        .await
        .next_result_execute_lpm_command
        .push_back(Ok(Some(disconnected_response)));
    execute_and_assert(
        service,
        get_connector_status_command(port_id),
        true,
        GlobalCci {
            cmd_complete: true,
            connector_change: GlobalPortId(port_id.0 + 1),
            ..Default::default()
        },
        Ok(Some(UcsiResponseData::Lpm(disconnected_response))),
    )
    .await;

    // Acknowledge the connector change.
    info!("Acknowledging CCI for port {}", port_id.0);
    ack_command_and_connector_change(service).await;
}

fn get_connector_status_command(port_id: GlobalPortId) -> GlobalCommand {
    GlobalCommand::LpmCommand(lpm::GlobalCommand::new(lpm::CommandData::GetConnectorStatus(
        lpm::get_connector_status::Args {
            connector_number: port_id,
        },
    )))
}

async fn ack_command_and_connector_change(service: &TypeCServiceMutexType<'_, '_>) {
    execute_and_assert(
        service,
        GlobalCommand::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: Ack {
                command_complete: true,
                connector_change: true,
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

fn port_capability() -> UcsiConnectorCapability {
    UcsiConnectorCapability {
        operation_mode: OperationModeFlags {
            drp: true,
            usb2: true,
            usb3: true,
            ..Default::default()
        },
        consumer: true,
        provider: true,
        swap_to_dfp: true,
        swap_to_snk: true,
        swap_to_src: true,
        ..Default::default()
    }
}

struct TestUcsi;

impl Test for TestUcsi {
    async fn run<'port, 'ch>(
        &mut self,
        service: &TypeCServiceMutexType<'port, 'ch>,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        _power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        port1: TestPort<'port, 'ch>,
        port2: TestPort<'port, 'ch>,
    ) {
        // Reset the PPM.
        info!("PPM reset");
        execute_and_assert(
            service,
            GlobalCommand::PpmCommand(ppm::Command::PpmReset),
            // OPM is expected to poll for the reset complete flag.
            false,
            GlobalCci {
                reset_complete: true,
                ..Default::default()
            },
            Ok(None),
        )
        .await;

        // Enable notifications.
        info!("Enabling notifications");
        let notifications = NotificationEnable {
            cmd_complete: true,
            connect_change: true,
            ..Default::default()
        };
        execute_and_assert(
            service,
            GlobalCommand::PpmCommand(ppm::Command::SetNotificationEnable(
                ppm::set_notification_enable::Args {
                    notification_enable: notifications,
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

        // Acknowledge the command completion.
        execute_and_assert(
            service,
            GlobalCommand::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
                ack: Ack {
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

        test_lpm(service, &type_c_receiver, &port0, GlobalPortId(0)).await;
        test_lpm(service, &type_c_receiver, &port1, GlobalPortId(1)).await;
        test_lpm(service, &type_c_receiver, &port2, GlobalPortId(2)).await;
    }
}

#[tokio::test]
async fn ucsi() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        tcpm_service::service::config::Config {
            ucsi_capabilities: PpmCapabilities {
                num_connectors: 3,
                bcd_usb_pd_spec: 0x0300,
                bcd_type_c_spec: 0x0200,
                bcd_battery_charging_spec: 0x0120,
                ..Default::default()
            },
            ucsi_port_capabilities: Some(port_capability()),
            ..Default::default()
        },
        Default::default(),
        TestUcsi,
    )
    .await;
}
