//! Integration tests for [`PortMock`] `plug`/`unplug` control helpers,
//! verifying that the appropriate Type-C and power-policy events are broadcast.
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use embassy_sync::channel::Channel;
use embedded_services::GlobalRawMutex;
use embedded_usb_pd::{PlugOrientation, PowerRole, type_c::ConnectionState};
use power_policy_interface::capability::{
    ConsumerFlags, ConsumerPowerCapability, PowerCapability, ProviderFlags, ProviderPowerCapability, PsuType,
};
use power_policy_interface::psu::event::EventData as PsuEventData;
use power_policy_interface::psu::{Psu, PsuState};
use type_c_interface::service::event::PortEventData;
use type_c_interface_mocks::port::{ConnectionConfig, PortMock, PortMockError};

const CHANNEL_SIZE: usize = 4;

const TEST_CAPABILITY: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 1500,
};

type TypeCChannel = Channel<GlobalRawMutex, PortEventData, CHANNEL_SIZE>;
type PowerChannel = Channel<GlobalRawMutex, PsuEventData, CHANNEL_SIZE>;

#[tokio::test]
async fn test_plug_sink_broadcasts_events() {
    let type_c_channel: TypeCChannel = Channel::new();
    let power_channel: PowerChannel = Channel::new();

    let mut mock = PortMock::new("test", type_c_channel.dyn_sender(), power_channel.dyn_sender());

    let config = ConnectionConfig {
        dual_power: true,
        plug_orientation: PlugOrientation::CC2,
    };
    mock.plug(PowerRole::Sink, TEST_CAPABILITY, config).unwrap();

    // Power policy should be notified of the attach
    assert_eq!(power_channel.try_receive().unwrap(), PsuEventData::Attached);
    assert!(power_channel.try_receive().is_err());

    // Type-C service should receive a status changed event reflecting the connection
    let PortEventData::StatusChanged(data) = type_c_channel.try_receive().unwrap() else {
        panic!("expected StatusChanged event");
    };
    assert!(data.status_event.plug_inserted_or_removed());
    assert!(!data.previous_status.is_connected());
    assert!(data.current_status.is_connected());
    assert_eq!(data.current_status.connection_state, Some(ConnectionState::Attached));
    assert_eq!(data.current_status.available_sink_contract, Some(TEST_CAPABILITY));
    assert_eq!(data.current_status.power_role, PowerRole::Sink);
    assert!(data.current_status.dual_power);
    assert_eq!(data.current_status.plug_orientation, PlugOrientation::CC2);
    assert!(type_c_channel.try_receive().is_err());

    // State should be Idle since power policy hasn't directed us to connect yet
    // But the consumer capability should have been recorded
    let expected_capability = ConsumerPowerCapability {
        capability: TEST_CAPABILITY,
        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
    };
    assert_eq!(mock.state().consumer_capability, Some(expected_capability));
    assert_eq!(mock.state().psu_state, PsuState::Idle);
    assert!(mock.status().is_connected());

    // Direct the mock to connect and verify state
    mock.connect_consumer(expected_capability).await.unwrap();
    assert_eq!(mock.state().psu_state, PsuState::ConnectedConsumer(expected_capability));
}

#[tokio::test]
async fn test_plug_source_broadcasts_events() {
    let type_c_channel: TypeCChannel = Channel::new();
    let power_channel: PowerChannel = Channel::new();

    let mut mock = PortMock::new("test", type_c_channel.dyn_sender(), power_channel.dyn_sender());

    mock.plug(PowerRole::Source, TEST_CAPABILITY, ConnectionConfig::default())
        .unwrap();

    assert_eq!(power_channel.try_receive().unwrap(), PsuEventData::Attached);

    let PortEventData::StatusChanged(data) = type_c_channel.try_receive().unwrap() else {
        panic!("expected StatusChanged event");
    };
    assert!(data.status_event.plug_inserted_or_removed());
    assert!(data.status_event.new_power_contract_as_provider());
    assert_eq!(data.current_status.available_source_contract, Some(TEST_CAPABILITY));
    assert_eq!(data.current_status.power_role, PowerRole::Source);

    // State should be Idle since power policy hasn't directed us to connect yet
    // But the requested provider capability should have been recorded
    let expected_capability = ProviderPowerCapability {
        capability: TEST_CAPABILITY,
        flags: ProviderFlags::none().with_psu_type(PsuType::TypeC),
    };
    assert_eq!(mock.state().requested_provider_capability, Some(expected_capability));
    assert_eq!(mock.state().psu_state, PsuState::Idle);
    assert!(mock.status().is_connected());

    // Direct the mock to connect and verify state
    mock.connect_provider(expected_capability).await.unwrap();
    assert_eq!(mock.state().psu_state, PsuState::ConnectedProvider(expected_capability));
}

#[tokio::test]
async fn test_unplug_broadcasts_events() {
    let type_c_channel: TypeCChannel = Channel::new();
    let power_channel: PowerChannel = Channel::new();

    let mut mock = PortMock::new("test", type_c_channel.dyn_sender(), power_channel.dyn_sender());

    // Establish a connection, then drain its events
    mock.plug(PowerRole::Sink, TEST_CAPABILITY, ConnectionConfig::default())
        .unwrap();
    while power_channel.try_receive().is_ok() {}
    while type_c_channel.try_receive().is_ok() {}

    mock.unplug().unwrap();

    // Power policy should be notified of the detach
    assert_eq!(power_channel.try_receive().unwrap(), PsuEventData::Detached);
    assert!(power_channel.try_receive().is_err());

    // Type-C service should receive a status changed event reflecting the removal
    let PortEventData::StatusChanged(data) = type_c_channel.try_receive().unwrap() else {
        panic!("expected StatusChanged event");
    };
    assert!(data.status_event.plug_inserted_or_removed());
    assert!(data.previous_status.is_connected());
    assert!(!data.current_status.is_connected());
    assert!(type_c_channel.try_receive().is_err());

    // Internal state should reflect the detached PSU
    assert_eq!(mock.state().psu_state, PsuState::Detached);
    assert!(!mock.status().is_connected());
}

#[tokio::test]
async fn test_unplug_without_connection_errors() {
    let type_c_channel: TypeCChannel = Channel::new();
    let power_channel: PowerChannel = Channel::new();

    let mut mock = PortMock::new("test", type_c_channel.dyn_sender(), power_channel.dyn_sender());

    // Unplugging a port that was never connected should error and broadcast nothing
    assert_eq!(mock.unplug(), Err(PortMockError::NotConnected));
    assert!(power_channel.try_receive().is_err());
    assert!(type_c_channel.try_receive().is_err());
}
