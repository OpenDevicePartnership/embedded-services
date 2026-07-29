use generic_array::ArrayLength;
use typenum::Max;

mod sealed {
    /// Traits that derive from this one are not allowed to be implemented by 3rd party code.
    /// To have those traits implemented, you should satisfy the requirement for their blanket
    /// implementation instead.
    pub trait Sealed {}
}

/// Extension of [`embedded_services::relay::hid::HidDevice`] that computes the max of feature/input and
/// feature/output report sizes as associated types so we can correctly size our
/// send/recv buffers.
///
/// Any type that implements `embedded_services::relay::hid::HidDevice` will automatically implement this trait -
/// there's no type that satisfies ArrayLength that doesn't also satisfy these trait bounds.
/// However, due to some limitations in the Rust type system, we have to spell it out.
///
/// We should be able to get rid of all of this once generic_const_exprs stabilises, since
/// then we don't need any of these trait bounds and can just do the math where we declare
/// the buffers. At that point, we should also consider moving from ArrayLength to just
/// const usizes since ArraySize is just a workaround for the lack of generic const expressions.
///
pub trait ConstrainedHidDevice: embedded_services::relay::hid::HidDevice + sealed::Sealed {
    /// `max(FeatureReportMaxSize, InputReportMaxSize)`.
    type MaxInputOrFeatureSize: ArrayLength;
    /// `max(FeatureReportMaxSize, OutputReportMaxSize)`.
    type MaxOutputOrFeatureSize: ArrayLength;
    /// `max(FeatureReportMaxSize, OutputReportMaxSize) + 9`.
    type WriteBufferSize: ArrayLength;
}

impl<T> ConstrainedHidDevice for T
where
    T: embedded_services::relay::hid::HidDevice,
    T::FeatureReportMaxSize: Max<T::InputReportMaxSize>,
    T::FeatureReportMaxSize: Max<T::OutputReportMaxSize>,
    <T::FeatureReportMaxSize as Max<T::InputReportMaxSize>>::Output: ArrayLength,
    <T::FeatureReportMaxSize as Max<T::OutputReportMaxSize>>::Output: ArrayLength,
    <T::FeatureReportMaxSize as Max<T::OutputReportMaxSize>>::Output: core::ops::Add<typenum::U9>,
    <<T::FeatureReportMaxSize as Max<T::OutputReportMaxSize>>::Output as core::ops::Add<typenum::U9>>::Output:
        ArrayLength,
{
    type MaxInputOrFeatureSize = <T::FeatureReportMaxSize as Max<T::InputReportMaxSize>>::Output;
    type MaxOutputOrFeatureSize = <T::FeatureReportMaxSize as Max<T::OutputReportMaxSize>>::Output;

    /// To avoid splitting the received values across multiple I2C read calls (which injects await points between them and manifests
    /// on the bus as clock stretching), we need to have a buffer that's large enough to consume the largest write that a host can do
    /// in a single transaction.
    ///
    /// In this case, that write is for handling the Command: SetReport path (note: not the 'normal' output report register, the one that
    /// goes through the Command/Data register path) - see section 7.2.3 of the HID spec.
    /// In that path, the largest possible write is:
    ///   2 bytes: command register address
    ///   2 bytes: command register value (SetReport)
    ///   1 byte: optional report ID extension to command register value for report IDs > 15
    ///   2 bytes: data register address
    ///   2 bytes: data register length header
    ///   N bytes: length of the actual report payload
    ///
    /// Therefore, this buffer needs to be 9 bytes larger than the largest output or feature report
    ///
    type WriteBufferSize =
        <<T::FeatureReportMaxSize as Max<T::OutputReportMaxSize>>::Output as core::ops::Add<typenum::U9>>::Output;
}

impl<T> sealed::Sealed for T where T: embedded_services::relay::hid::HidDevice {}
