//! Code related to registration with the power policy service.

use embedded_services::sync::Lockable;
use power_policy_interface::{charger, psu};

/// Registration trait that abstracts over various registration details.
pub trait Registration<'device> {
    type Psu: Lockable<Inner: psu::Psu> + 'device;
    type ServiceNotifier: power_policy_interface::service::notification::Notifier<'device, Psu = Self::Psu>;
    type Charger: Lockable<Inner: charger::Charger> + 'device;

    /// Returns a slice to access PSU devices
    fn psus(&self) -> &[&'device Self::Psu];
    /// Returns a slice to access power policy notifiers
    fn notifiers(&mut self) -> &mut [Self::ServiceNotifier];
    /// Returns a slice to access charger devices
    fn chargers(&self) -> &[&'device Self::Charger];
}

/// A registration implementation based around arrays
pub struct ArrayRegistration<
    'device,
    Psu: Lockable<Inner: psu::Psu> + 'device,
    const PSU_COUNT: usize,
    ServiceNotifier: power_policy_interface::service::notification::Notifier<'device, Psu = Psu>,
    const SERVICE_NOTIFIER_COUNT: usize,
    Charger: Lockable<Inner: charger::Charger> + 'device,
    const CHARGER_COUNT: usize,
> {
    /// Array of registered PSUs
    pub psus: [&'device Psu; PSU_COUNT],
    /// Array of registered chargers
    pub chargers: [&'device Charger; CHARGER_COUNT],
    /// Array of power policy service notifiers
    pub service_notifiers: [ServiceNotifier; SERVICE_NOTIFIER_COUNT],
}

impl<
    'device,
    Psu: Lockable<Inner: psu::Psu> + 'device,
    const PSU_COUNT: usize,
    ServiceNotifier: power_policy_interface::service::notification::Notifier<'device, Psu = Psu>,
    const SERVICE_NOTIFIER_COUNT: usize,
    Charger: Lockable<Inner: charger::Charger> + 'device,
    const CHARGER_COUNT: usize,
> Registration<'device>
    for ArrayRegistration<'device, Psu, PSU_COUNT, ServiceNotifier, SERVICE_NOTIFIER_COUNT, Charger, CHARGER_COUNT>
{
    type Psu = Psu;
    type ServiceNotifier = ServiceNotifier;
    type Charger = Charger;

    fn psus(&self) -> &[&'device Self::Psu] {
        &self.psus
    }

    fn notifiers(&mut self) -> &mut [Self::ServiceNotifier] {
        &mut self.service_notifiers
    }

    fn chargers(&self) -> &[&'device Self::Charger] {
        &self.chargers
    }
}
