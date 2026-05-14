//! Code related to registration with the type-C service

use embedded_services::{event::Sender, sync::Lockable};
use embedded_usb_pd::{GlobalPortId, LocalPortId};
use type_c_interface::port::pd::Pd;
use type_c_interface::service::event::Event as ServiceEvent;
use type_c_interface::ucsi::Lpm as UcsiLpm;

/// Registration trait that abstracts over various registration details.
pub trait Registration<'port> {
    type Port: Lockable<Inner: Pd + UcsiLpm> + 'port;
    type ServiceSender: Sender<ServiceEvent<'port, Self::Port>>;

    /// Returns a slice to access ports
    fn ports(&self) -> &[&'port Self::Port];
    /// Returns a slice to access type-c event senders
    fn event_senders(&mut self) -> &mut [Self::ServiceSender];
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
    ServiceSender: Sender<ServiceEvent<'port, Port>>,
    const SERVICE_SENDER_COUNT: usize,
> {
    /// Array of registered ports
    pub ports: [&'port Port; PORT_COUNT],
    /// Array of local port data
    pub port_data: [PortData; PORT_COUNT],
    /// Array of service event senders
    pub service_senders: [ServiceSender; SERVICE_SENDER_COUNT],
}

impl<
    'port,
    Port: Lockable<Inner: Pd + UcsiLpm> + 'port,
    const PORT_COUNT: usize,
    ServiceSender: Sender<ServiceEvent<'port, Port>>,
    const SERVICE_SENDER_COUNT: usize,
> Registration<'port> for ArrayRegistration<'port, Port, PORT_COUNT, ServiceSender, SERVICE_SENDER_COUNT>
{
    type Port = Port;
    type ServiceSender = ServiceSender;

    fn event_senders(&mut self) -> &mut [Self::ServiceSender] {
        &mut self.service_senders
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
