#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// ODP Specific Debug Commands
pub enum DebugCmd {
    /// Get buffer of debug messages, if available.
    /// Can be used to poll debug messages.
    GetMsgs = 1,
}
