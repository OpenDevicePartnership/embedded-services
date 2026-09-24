//! Code related to registration with the type-C service

use embedded_services::sync::Lockable;
use embedded_usb_pd::{GlobalPortId, LocalPortId};
use tcpm_interface::port::pd::Pd;
use tcpm_interface::service::notification::Notifier as ServiceNotifierTrait;
use tcpm_interface::ucsi::Lpm as UcsiLpm;

/// Registration trait that abstracts over various registration details.
pub trait Registration<'port> {
    type Port: Lockable<Inner: Pd + UcsiLpm> + 'port;
    type ServiceNotifier: ServiceNotifierTrait<'port, Port = Self::Port>;

    /// Returns a slice to access ports
    fn ports(&self) -> &[&'port Self::Port];
    /// Returns a slice to access type-c service notifiers
    fn notifiers(&mut self) -> &mut [Self::ServiceNotifier];
    /// Returns the ucsi local port ID for a given global port
    fn ucsi_local_port_id(&self, global_port: GlobalPortId) -> Option<LocalPortId>;
}

pub struct PortData {
    /// local port ID
    pub local_port: Option<LocalPortId>,
}

/// A registration implementation based around arrays
pub struct ArrayRegistration<
    'port,
    Port: Lockable<Inner: Pd + UcsiLpm> + 'port,
    const PORT_COUNT: usize,
    ServiceNotifier: ServiceNotifierTrait<'port, Port = Port>,
    const SERVICE_NOTIFIER_COUNT: usize,
> {
    /// Array of registered ports
    pub ports: [&'port Port; PORT_COUNT],
    /// Array of local port data
    pub port_data: [PortData; PORT_COUNT],
    /// Array of service event notifiers
    pub service_notifiers: [ServiceNotifier; SERVICE_NOTIFIER_COUNT],
}

impl<
    'port,
    Port: Lockable<Inner: Pd + UcsiLpm> + 'port,
    const PORT_COUNT: usize,
    ServiceNotifier: ServiceNotifierTrait<'port, Port = Port>,
    const SERVICE_NOTIFIER_COUNT: usize,
> Registration<'port> for ArrayRegistration<'port, Port, PORT_COUNT, ServiceNotifier, SERVICE_NOTIFIER_COUNT>
{
    type Port = Port;
    type ServiceNotifier = ServiceNotifier;

    fn notifiers(&mut self) -> &mut [Self::ServiceNotifier] {
        &mut self.service_notifiers
    }

    fn ports(&self) -> &[&'port Self::Port] {
        &self.ports
    }

    fn ucsi_local_port_id(&self, global_port: GlobalPortId) -> Option<LocalPortId> {
        self.port_data
            .get(global_port.0 as usize)
            .and_then(|data| data.local_port)
    }
}
