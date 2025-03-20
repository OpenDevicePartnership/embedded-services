//! Logging macro implementations and other formating functions

#[cfg(all(feature = "log", feature = "defmt", not(doc)))]
compile_error!("features `log` and `defmt` are mutually exclusive");

#[cfg(all(not(doc), feature = "defmt"))]
mod defmt {
    /// Logs a trace message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! trace {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::defmt::trace!($s $(, $x)*);
            }
        };
    }

    /// Logs a debug message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! debug {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::defmt::debug!($s $(, $x)*);
            }
        };
    }

    /// Logs an info message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! info {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::defmt::info!($s $(, $x)*);
            }
        };
    }

    /// Logs a warning using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! warn {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::defmt::warn!($s $(, $x)*);
            }
        };
    }

    /// Logs an error using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! error {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::defmt::error!($s $(, $x)*);
            }
        };
    }
}

#[cfg(all(not(doc), feature = "log"))]
mod log {
    /// Logs a trace message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! trace {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::log::trace!($s $(, $x)*);
            }
        };
    }

    /// Logs a debug message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! debug {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::log::debug!($s $(, $x)*);
            }
        };
    }

    /// Logs an info message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! info {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::log::info!($s $(, $x)*);
            }
        };
    }

    /// Logs a warning using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! warn {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::log::warn!($s $(, $x)*);
            }
        };
    }

    /// Logs an error using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! error {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                ::log::error!($s $(, $x)*);
            }
        };
    }
}

// Provide this implementation for `cargo doc`
#[cfg(any(doc, not(any(feature = "defmt", feature = "log"))))]
mod none {
    /// Logs a trace message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! trace {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
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
                let _ = ($( & $x ),*);
            }
        };
    }

    /// Logs an info message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! info {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
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
                let _ = ($( & $x ),*);
            }
        };
    }
}
