//! Fuel gauge registration with the battery service.
//!
//! The [`Registration`] trait abstracts over how a set of fuel gauges is provided to
//! the service, and [`ArrayRegistration`] is a simple array-backed
//! implementation. The OEM owns each fuel gauge (typically behind an
//! `embassy_sync` `Mutex`) and drives it directly via the
//! [`FuelGauge`](battery_service_interface::fuel_gauge::FuelGauge) trait methods; the
//! battery service only reads each fuel gauge's cached state to answer ACPI
//! queries.

use battery_service_interface::DeviceId;
use embedded_services::sync::Lockable;

/// Registration trait that abstracts over how fuel gauges are provided to the service.
pub trait Registration<'hw> {
    /// The lockable fuel gauge type. Its inner type implements
    /// [`FuelGauge`](battery_service_interface::fuel_gauge::FuelGauge).
    type FuelGauge: Lockable<Inner: battery_service_interface::fuel_gauge::FuelGauge> + 'hw;

    /// Returns a slice of the registered fuel gauges.
    ///
    /// The position of a fuel gauge in this slice is its
    /// [`DeviceId`](battery_service_interface::DeviceId) for ACPI queries (the
    /// first registered fuel gauge is battery `0`, and so on).
    fn fuel_gauges(&self) -> &[&'hw Self::FuelGauge];

    /// Look up a registered fuel gauge by its device ID.
    ///
    /// The device ID is the fuel gauge's position in the registration (the first
    /// registered fuel gauge is battery `0`, and so on).
    fn get_fuel_gauge(&self, id: DeviceId) -> Option<&'hw Self::FuelGauge> {
        self.fuel_gauges().get(usize::from(id.0)).copied()
    }
}

/// An array-backed [`Registration`] implementation.
pub struct ArrayRegistration<
    'hw,
    FG: Lockable<Inner: battery_service_interface::fuel_gauge::FuelGauge> + 'hw,
    const N: usize,
> {
    /// The registered fuel gauges.
    pub fuel_gauges: [&'hw FG; N],
}

impl<'hw, FG: Lockable<Inner: battery_service_interface::fuel_gauge::FuelGauge> + 'hw, const N: usize> Registration<'hw>
    for ArrayRegistration<'hw, FG, N>
{
    type FuelGauge = FG;

    fn fuel_gauges(&self) -> &[&'hw Self::FuelGauge] {
        &self.fuel_gauges
    }
}
