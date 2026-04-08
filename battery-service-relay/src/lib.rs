#![no_std]

use battery_service_interface::*;
use embedded_services::trace;

mod serialization;
pub use serialization::{AcpiBatteryError, AcpiBatteryRequest, AcpiBatteryResponse, AcpiBatteryResult};

/// Relays messages to and from a battery service implementation over MCTP.
pub struct BatteryServiceRelayHandler<S: battery_service_interface::BatteryService> {
    service: S,
}

impl<S: battery_service_interface::BatteryService> BatteryServiceRelayHandler<S> {
    /// Create a new relay handler that uses the provided battery service implementation to handle requests.
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

impl<S: battery_service_interface::BatteryService> embedded_services::relay::mctp::RelayServiceHandlerTypes
    for BatteryServiceRelayHandler<S>
{
    type RequestType = serialization::AcpiBatteryRequest;
    type ResultType = serialization::AcpiBatteryResult;
}

impl<S: battery_service_interface::BatteryService> embedded_services::relay::mctp::RelayServiceHandler
    for BatteryServiceRelayHandler<S>
{
    async fn process_request(&self, request: Self::RequestType) -> Self::ResultType {
        trace!("Battery service: ACPI cmd recvd");
        Ok(match request {
            AcpiBatteryRequest::GetBix { battery_id } => AcpiBatteryResponse::GetBix {
                bix: self.service.battery_info(DeviceId(battery_id)).await?,
            },
            AcpiBatteryRequest::GetBst { battery_id } => AcpiBatteryResponse::GetBst {
                bst: self.service.battery_status(DeviceId(battery_id)).await?,
            },
            AcpiBatteryRequest::GetPsr { battery_id } => AcpiBatteryResponse::GetPsr {
                psr: self.service.is_in_use(DeviceId(battery_id)).await?,
            },
            AcpiBatteryRequest::GetPif { battery_id } => AcpiBatteryResponse::GetPif {
                pif: self.service.power_source_information(DeviceId(battery_id)).await?,
            },
            AcpiBatteryRequest::GetBps { battery_id } => AcpiBatteryResponse::GetBps {
                bps: self.service.battery_power_state(DeviceId(battery_id)).await?,
            },
            AcpiBatteryRequest::SetBtp { battery_id, btp } => {
                self.service.set_battery_trip_point(DeviceId(battery_id), btp).await?;
                AcpiBatteryResponse::SetBtp {}
            }
            AcpiBatteryRequest::SetBpt { battery_id, bpt } => {
                self.service
                    .set_battery_power_threshold(DeviceId(battery_id), bpt)
                    .await?;
                AcpiBatteryResponse::SetBpt {}
            }

            AcpiBatteryRequest::GetBpc { battery_id } => AcpiBatteryResponse::GetBpc {
                bpc: self.service.battery_power_characteristics(DeviceId(battery_id)).await?,
            },
            AcpiBatteryRequest::SetBmc { battery_id, bmc } => {
                self.service
                    .battery_maintenance_control(DeviceId(battery_id), bmc)
                    .await?;
                AcpiBatteryResponse::SetBmc {}
            }
            AcpiBatteryRequest::GetBmd { battery_id } => AcpiBatteryResponse::GetBmd {
                bmd: self.service.battery_maintenance_data(DeviceId(battery_id)).await?,
            },
            AcpiBatteryRequest::GetBct { battery_id, bct } => AcpiBatteryResponse::GetBct {
                bct_response: self.service.battery_charge_time(DeviceId(battery_id), bct).await?,
            },
            AcpiBatteryRequest::GetBtm { battery_id, btm } => AcpiBatteryResponse::GetBtm {
                btm_response: self.service.battery_time_to_empty(DeviceId(battery_id), btm).await?,
            },

            AcpiBatteryRequest::SetBms { battery_id, bms } => {
                self.service
                    .set_battery_measurement_sampling_time(DeviceId(battery_id), bms)
                    .await?;
                AcpiBatteryResponse::SetBms {
                    status: 0, // TODO once we have a working reference platform, we should consider dropping this field, since it's redundant with the error type on Result.
                }
            }
            AcpiBatteryRequest::SetBma { battery_id, bma } => {
                self.service
                    .set_battery_measurement_averaging_interval(DeviceId(battery_id), bma)
                    .await?;
                AcpiBatteryResponse::SetBma {
                    status: 0, // TODO once we have a working reference platform, we should consider dropping this field, since it's redundant with the error type on Result.
                }
            }
            AcpiBatteryRequest::GetSta { battery_id } => AcpiBatteryResponse::GetSta {
                sta: self.service.device_status(DeviceId(battery_id)).await?,
            },
        })
    }
}
