use embedded_services::{
    comms,
    relay::{SerializableMessage, SerializableResult},
};

// TODO we're currently transitioning from the comms service to direct async calls to support better testing, reduced code size, and better performance.
//      As part of this transition, each service that interacts with the eSPI service needs to migrate to expose a direct async call API and implement
//      some additional traits to be able to interface with relay services in the new way.
//
//      Until all services have been migrated, we need to support both the old and new methods for interfacing with services.  Once migration is complete,
//      we can remove all the legacy code that supports the old comms service method of interfacing with the eSPI service.
//      These are the ones that have not been migrated to using the new method.  When a service is migrated, remove it here and pass it into
//      the new impl macro from the 'application' layer.
//
//      You should not add any new types here. Instead, implement the embedded_services::relay::mctp::RelayServiceHandler trait for your service.
//
#[allow(deprecated)]
mod legacy_relay {
    use super::*;
    use embedded_services::relay::mctp::impl_odp_mctp_relay_types;
    impl_odp_mctp_relay_types!(
        Battery,   0x08, (comms::EndpointID::Internal(comms::Internal::Battery)), battery_service_messages::AcpiBatteryRequest, battery_service_messages::AcpiBatteryResult;
        Thermal,   0x09, (comms::EndpointID::Internal(comms::Internal::Thermal)), thermal_service_messages::ThermalRequest,     thermal_service_messages::ThermalResult;
        Debug,     0x0A, (comms::EndpointID::Internal(comms::Internal::Debug)),   debug_service_messages::DebugRequest,         debug_service_messages::DebugResult;
    );
}
pub(crate) use legacy_relay::*;
