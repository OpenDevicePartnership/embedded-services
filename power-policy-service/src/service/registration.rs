//! Code related to registration with the power policy service.
use embedded_services::{event::Sender, sync::Lockable};
use power_policy_interface::{psu, service::event::Event as ServiceEvent};

/// Registration trait that abstracts over various registration details.
pub trait Registration<'device> {
    type Psu: Lockable<Inner: psu::Psu> + 'device;
    type ServiceSender: Sender<ServiceEvent<'device, Self::Psu>>;

    /// Returns a slice to access PSU devices
    fn psus(&self) -> &[&'device Self::Psu];
    /// Returns a slice to access power policy event senders
    fn event_senders(&mut self) -> &mut [Self::ServiceSender];
}

/// A registration implementation based around arrays
pub struct ArrayRegistration<
    'device,
    Psu: Lockable<Inner: psu::Psu> + 'device,
    const PSU_COUNT: usize,
    ServiceSender: Sender<ServiceEvent<'device, Psu>>,
    const SERVICE_SENDER_COUNT: usize,
> {
    /// Array of registered PSUs
    pub psus: [&'device Psu; PSU_COUNT],
    /// Array of power policy service event senders
    pub service_senders: [ServiceSender; SERVICE_SENDER_COUNT],
}

impl<
    'device,
    Psu: Lockable<Inner: psu::Psu> + 'device,
    const PSU_COUNT: usize,
    ServiceSender: Sender<ServiceEvent<'device, Psu>>,
    const SERVICE_SENDER_COUNT: usize,
> Registration<'device> for ArrayRegistration<'device, Psu, PSU_COUNT, ServiceSender, SERVICE_SENDER_COUNT>
{
    type Psu = Psu;
    type ServiceSender = ServiceSender;

    fn psus(&self) -> &[&'device Self::Psu] {
        &self.psus
    }

    fn event_senders(&mut self) -> &mut [Self::ServiceSender] {
        &mut self.service_senders
    }
}
