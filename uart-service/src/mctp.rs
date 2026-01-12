use embedded_services::{
    comms,
    relay::{SerializableMessage, SerializableResult, mctp::impl_odp_mctp_relay_types},
};

// TODO We'd ideally like these types to be passed in as a generic or something when the UART service is instantiated
//      so the UART service can be extended to handle 3rd party message types without needing to fork the UART service
impl_odp_mctp_relay_types!(
    Battery, 0x08, (comms::EndpointID::Internal(comms::Internal::Battery)), battery_service_messages::AcpiBatteryRequest, battery_service_messages::AcpiBatteryResult;
    Thermal, 0x09, (comms::EndpointID::Internal(comms::Internal::Thermal)), thermal_service_messages::ThermalRequest, thermal_service_messages::ThermalResult;
    Debug, 0x0A,   (comms::EndpointID::Internal(comms::Internal::Debug)  ), debug_service_messages::DebugRequest, debug_service_messages::DebugResult;
);
