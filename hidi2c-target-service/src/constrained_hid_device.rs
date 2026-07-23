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
}

impl<T> ConstrainedHidDevice for T
where
    T: embedded_services::relay::hid::HidDevice,
    T::FeatureReportMaxSize: Max<T::InputReportMaxSize>,
    T::FeatureReportMaxSize: Max<T::OutputReportMaxSize>,
    <T::FeatureReportMaxSize as Max<T::InputReportMaxSize>>::Output: ArrayLength,
    <T::FeatureReportMaxSize as Max<T::OutputReportMaxSize>>::Output: ArrayLength,
{
    type MaxInputOrFeatureSize = <T::FeatureReportMaxSize as Max<T::InputReportMaxSize>>::Output;
    type MaxOutputOrFeatureSize = <T::FeatureReportMaxSize as Max<T::OutputReportMaxSize>>::Output;
}

impl<T> sealed::Sealed for T where T: embedded_services::relay::hid::HidDevice {}
