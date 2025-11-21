/// Configuration for Type-C controller wrapper
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Config {
    /// Unconstrained behavior for sink role
    pub unconstrained_sink: UnconstrainedSink,
}

/// Unconstrained behavior for sink role
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum UnconstrainedSink {
    /// Automatically signal unconstrained power based on unconstrained bit in PDO
    #[default]
    Auto,
    /// Automatically signal unconstrained power for any sink that meets a power threshold in mW
    PowerThresholdMilliwatts(u32),
    /// Never signal unconstrained power
    Never,
}
