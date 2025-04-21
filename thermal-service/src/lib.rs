#![no_std]

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::{comms, error, info};
pub use thermal_zone::*;

pub mod fan;
pub mod mptf;
pub mod sensor;
pub mod thermal_zone;

/// Contains information concerning where to route unknown messages (dictated by the supplied OemKey)
/// and if the service should route standard MPTF messages to the OEM or handle them itself.
///
/// A better strategy will likely be implemented, but this is the general idea.
pub struct Oem {
    route: comms::OemKey,
    route_mptf: bool,
}

impl Oem {
    pub fn new(route: comms::OemKey, route_mptf: bool) -> Self {
        Self { route, route_mptf }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ServiceMsg {
    pub msg: mptf::Request,
    pub from: comms::EndpointID,
}

impl ServiceMsg {
    pub fn new(msg: mptf::Request, from: comms::EndpointID) -> Self {
        Self { msg, from }
    }
}

pub struct ThermalService<T: ThermalZone> {
    endpoint: comms::Endpoint,
    request: Channel<NoopRawMutex, ServiceMsg, 1>,
    oem: Oem,
    tz: Mutex<NoopRawMutex, T>,
}

impl<T: ThermalZone> ThermalService<T> {
    fn new(tz: T, oem: Oem) -> Option<Self> {
        Some(Self {
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Thermal)),
            request: Channel::new(),
            oem,
            tz: Mutex::new(tz),
        })
    }

    // Process a received standard MPTF request
    async fn process_mptf_request(&self, service_msg: mptf::Request) -> Result<mptf::Response, mptf::Error> {
        let tz = self.tz.lock().await;

        match service_msg {
            // Standard command
            mptf::Request::GetTmp(_) => tz.get_tmp().await,
            mptf::Request::GetThrs(_) => tz.get_thrs().await,
            mptf::Request::SetThrs(_, timeout, low, high) => tz.set_thrs(timeout, low, high).await,
            mptf::Request::SetScp(_, policy, acstc_limit, pow_limit) => {
                tz.set_scp(policy, acstc_limit, pow_limit).await
            }

            // DWORD Variable - Thermal
            mptf::Request::GetCrtTemp => tz.get_crt_temp().await,
            mptf::Request::SetCrtTemp(temp) => tz.set_crt_temp(temp).await,
            mptf::Request::GetProcHotTemp => tz.get_proc_hot_temp().await,
            mptf::Request::SetProcHotTemp(temp) => tz.set_proc_hot_temp(temp).await,
            mptf::Request::GetProfileType => tz.get_profile_type().await,
            mptf::Request::SetProfileType(profile_type) => tz.set_profile_type(profile_type).await,

            // DWORD Variable - Fan
            mptf::Request::GetFanOnTemp => tz.get_fan_on_temp().await,
            mptf::Request::SetFanOnTemp(temp) => tz.set_fan_on_temp(temp).await,
            mptf::Request::GetFanRampTemp => tz.get_fan_ramp_temp().await,
            mptf::Request::SetFanRampTemp(temp) => tz.set_fan_ramp_temp(temp).await,
            mptf::Request::GetFanMaxTemp => tz.get_fan_max_temp().await,
            mptf::Request::SetFanMaxTemp(temp) => tz.set_fan_max_temp(temp).await,
            mptf::Request::GetFanMinRpm => tz.get_fan_min_rpm().await,
            mptf::Request::GetFanMaxRpm => tz.get_fan_max_rpm().await,
            mptf::Request::GetFanCurrentRpm => tz.get_fan_current_rpm().await,

            // DWORD Variable - Fan Optional
            mptf::Request::GetFanMinDba => tz.get_fan_min_dba().await,
            mptf::Request::GetFanMaxDba => tz.get_fan_max_dba().await,
            mptf::Request::GetFanCurrentDba => tz.get_fan_current_dba().await,
            mptf::Request::GetFanMinSones => tz.get_fan_min_sones().await,
            mptf::Request::GetFanMaxSones => tz.get_fan_max_sones().await,
            mptf::Request::GetFanCurrentSones => tz.get_fan_current_sones().await,
        }
    }

    async fn wait_service_msg(&self) -> ServiceMsg {
        self.request.receive().await
    }

    async fn wait_and_process(&self) {
        let request = self.wait_service_msg().await;
        let response = self.process_mptf_request(request.msg).await;
        self.endpoint.send(request.from, &response).await.unwrap()
    }
}

impl<T: ThermalZone> comms::MailboxDelegate for ThermalService<T> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // This method gets called by embedded-services any time a message is sent to this service
        // We check if its a standard MPTF request, and if so, handle it ourselves (unless OEM wants to handle them)
        // Unknown messages get routed to the OEM as well
        //
        // Unfortunately since this method isn't async can't use the async endpoint.send() method here
        // So have to come up with a cleaner way to move the message to move non-MPTF messages to a generic buffer
        // for later forwarding by a different async task.
        if !self.oem.route_mptf {
            // If MPTF request, send to our request channel for processing
            if let Some(&msg) = message.data.get::<mptf::Request>() {
                self.request
                    .try_send(ServiceMsg::new(msg, message.from))
                    .map_err(|_| comms::MailboxDelegateError::BufferFull)
            } else {
                // TODO: Route unknown message to OEM
                info!("Routing to: {}", self.oem.route);
                todo!()
            }
        } else {
            // TODO: Always route message to OEM
            info!("Routing to: {}", self.oem.route);
            todo!()
        }
    }
}

// Just one instance of the service should be running
// TODO: Have to reconsider because this can't be made generic over any impl ThermalZone
static SERVICE: OnceLock<ThermalService<&'static GenericThermalZone>> = OnceLock::new();

// This task exists solely to listen for incoming requests and then process them appropriately
#[embassy_executor::task]
pub async fn rx_task() {
    let s = SERVICE.get().await;

    loop {
        s.wait_and_process().await;
    }
}

/// This must be called to initialize the Thermal service and spawn additional tasks
pub async fn init(spawner: embassy_executor::Spawner, tz: &'static GenericThermalZone, oem: Oem) {
    info!("Starting thermal service task");
    let service =
        SERVICE.get_or_init(|| ThermalService::new(tz, oem).expect("Thermal service singleton already initialized"));

    if comms::register_endpoint(service, &service.endpoint).await.is_err() {
        error!("Failed to register thermal service endpoint");
        return;
    }

    // If the OEM doesn't want to handle MPTF logic, spawn the generic ThermalZone task
    if !service.oem.route_mptf {
        spawner.must_spawn(thermal_zone::generic_task(SERVICE.get().await, tz));
    }

    // But always spawn the task for receiving thermal messages
    spawner.must_spawn(rx_task());
}
