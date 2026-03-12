//! Traits for things that have names.

/// Trait for anything that has a name.
pub trait Named {
    /// Return name
    fn name(&self) -> &'static str;
}
