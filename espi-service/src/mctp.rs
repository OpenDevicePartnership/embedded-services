use embedded_services::{
    comms,
    relay::{SerializableMessage, SerializableResult, mctp::impl_odp_mctp_relay_types},
};

// TODO We'd ideally like these types to be passed in as a generic or something when the eSPI service is instantiated
//      so the eSPI service can be extended to handle 3rd party message types without needing to fork the eSPI service,
//      but that's dependant on us migrating to have storage for the eSPI service be allocated by the caller of init()
//      rather than statically allocated inside this module, so for now we accept this hardcoded list of supported message
//      types.

impl_odp_mctp_relay_types!(
    Battery,   0x08, (comms::EndpointID::Internal(comms::Internal::Battery)),   battery_service_messages::AcpiBatteryRequest,      battery_service_messages::AcpiBatteryResult;
    Thermal,   0x09, (comms::EndpointID::Internal(comms::Internal::Thermal)),   thermal_service_messages::ThermalRequest,          thermal_service_messages::ThermalResult;
    Debug,     0x0A, (comms::EndpointID::Internal(comms::Internal::Debug)),     debug_service_messages::DebugRequest,              debug_service_messages::DebugResult;
    TimeAlarm, 0x0B, (comms::EndpointID::Internal(comms::Internal::TimeAlarm)), time_alarm_service_messages::AcpiTimeAlarmRequest, time_alarm_service_messages::AcpiTimeAlarmResult;
);
