//! Logging macro implementations and other formating functions

/// Logs a trace message using the underlying logger
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! trace {
    ($s:literal $(, $x:expr)* $(,)?) => {
        {
            #[cfg(feature = "defmt")]
            ::defmt::trace!($s $(, $x)*);
            #[cfg(feature = "log")]
            ::log::trace!($s $(, $x)*);
            #[cfg(not(any(feature = "defmt", feature = "log")))]
            let _ = ($( & $x ),*);
        }
    };
}

/// Logs a debug message using the underlying logger
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! debug {
    ($s:literal $(, $x:expr)* $(,)?) => {
        {
            #[cfg(feature = "defmt")]
            ::defmt::debug!($s $(, $x)*);
            #[cfg(feature = "log")]
            ::log::debug!($s $(, $x)*);
            #[cfg(not(any(feature = "defmt", feature = "log")))]
            let _ = ($( & $x ),*);
        }
    };
}

/// Logs a info message using the underlying logger
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! info {
    ($s:literal $(, $x:expr)* $(,)?) => {
        {
            #[cfg(feature = "defmt")]
            ::defmt::info!($s $(, $x)*);
            #[cfg(feature = "log")]
            ::log::info!($s $(, $x)*);
            #[cfg(not(any(feature = "defmt", feature = "log")))]
            let _ = ($( & $x ),*);
        }
    };
}

/// Logs a warning using the underlying logger
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! warn {
    ($s:literal $(, $x:expr)* $(,)?) => {
        {
            #[cfg(feature = "defmt")]
            ::defmt::warn!($s $(, $x)*);
            #[cfg(feature = "log")]
            ::log::warn!($s $(, $x)*);
            #[cfg(not(any(feature = "defmt", feature = "log")))]
            let _ = ($( & $x ),*);
        }
    };
}

/// Logs an error using the underlying logger
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! error {
    ($s:literal $(, $x:expr)* $(,)?) => {
        {
            #[cfg(feature = "defmt")]
            ::defmt::error!($s $(, $x)*);
            #[cfg(feature = "log")]
            ::log::error!($s $(, $x)*);
            #[cfg(not(any(feature = "defmt", feature = "log")))]
            let _ = ($( & $x ),*);
        }
    };
}
