//! Logging macro implementations and other formating functions

// In no_std (embedded) targets, defmt and log are mutually exclusive to avoid double logging
#[cfg(all(feature = "log", feature = "defmt", target_os = "none", not(doc)))]
compile_error!("features `log` and `defmt` are mutually exclusive on no_std targets");

// In std/host targets, allow emitting to both defmt and log when both features are enabled
#[cfg(all(not(doc), feature = "defmt", feature = "log", not(target_os = "none")))]
mod defmt_and_log {
    /// Logs a trace message using both defmt and log
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! trace {
        ($s:literal $(, $x:expr)* $(,)?) => {{
            let _ = ($s $(, &$x )*);
            ::defmt::trace!($s $(, $x)*);
            ::log::trace!($s $(, $x)*);
        }};
    }

    /// Logs a debug message using both defmt and log
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! debug {
        ($s:literal $(, $x:expr)* $(,)?) => {{
            let _ = ($s $(, &$x )*);
            ::defmt::debug!($s $(, $x)*);
            ::log::debug!($s $(, $x)*);
        }};
    }

    /// Logs an info message using both defmt and log
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! info {
        ($s:literal $(, $x:expr)* $(,)?) => {{
            let _ = ($s $(, &$x )*);
            ::defmt::info!($s $(, $x)*);
            ::log::info!($s $(, $x)*);
        }};
    }

    /// Logs a warning using both defmt and log
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! warn {
        ($s:literal $(, $x:expr)* $(,)?) => {{
            let _ = ($s $(, &$x )*);
            ::defmt::warn!($s $(, $x)*);
            ::log::warn!($s $(, $x)*);
        }};
    }

    /// Logs an error using both defmt and log
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! error {
        ($s:literal $(, $x:expr)* $(,)?) => {{
            let _ = ($s $(, &$x )*);
            ::defmt::error!($s $(, $x)*);
            ::log::error!($s $(, $x)*);
        }};
    }
}

#[cfg(all(not(doc), feature = "defmt", not(all(feature = "log", not(target_os = "none")))))]
mod defmt {
    /// Logs a trace message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! trace {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                let _ = ($s, $( &$x ),*);
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
                let _ = ($s, $( &$x ),*);
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
                let _ = ($s, $( &$x ),*);
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
                let _ = ($s, $( &$x ),*);
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
                let _ = ($s, $( &$x ),*);
                ::defmt::error!($s $(, $x)*);
            }
        };
    }
}

#[cfg(all(not(doc), feature = "log", not(all(feature = "defmt", not(target_os = "none")))))]
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
                let _ = ($s, $( &$x ),*);
            }
        };
    }

    /// Logs a debug message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! debug {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                let _ = ($s, $( &$x ),*);
            }
        };
    }

    /// Logs an info message using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! info {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                let _ = ($s, $( &$x ),*);
            }
        };
    }

    /// Logs a warning using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! warn {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                let _ = ($s, $( &$x ),*);
            }
        };
    }

    /// Logs an error using the underlying logger
    #[macro_export]
    #[collapse_debuginfo(yes)]
    macro_rules! error {
        ($s:literal $(, $x:expr)* $(,)?) => {
            {
                let _ = ($s, $( &$x ),*);
            }
        };
    }
}
