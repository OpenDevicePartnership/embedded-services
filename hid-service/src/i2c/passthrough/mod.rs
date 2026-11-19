mod interrupt;

pub use interrupt::*;

/// Define the physical-device task with a compile-time response-buffer capacity.
///
/// The legacy one-argument form keeps its 512-byte allocation. Supply
/// `buffer_size = N` to support a device with larger descriptors or reports.
#[macro_export]
macro_rules! define_i2c_passthrough_device_task {
    ($bus:ty) => {
        $crate::define_i2c_passthrough_device_task!($bus, buffer_size = 512);
    };

    ($bus:ty, buffer_size = $buffer_size:expr $(,)?) => {
        pub async fn device_task(bus: $bus, id: ::embedded_services::hid::DeviceId, addr: u8) {
            use ::embassy_sync::once_lock::OnceLock;
            use ::embedded_services::{define_static_buffer, error, hid, info};
            use $crate::i2c::{Device, DeviceConfig};

            define_static_buffer!(gen_buffer, u8, [0; $buffer_size]);
            let gen_buffer = gen_buffer::get_mut().unwrap();

            info!("Create HID passthrough device {}", id.0);
            static DEVICE: OnceLock<Device<u8, $bus>> = OnceLock::new();
            let device = DEVICE.get_or_init(|| {
                Device::new(
                    id,
                    addr,
                    bus,
                    Default::default(),
                    gen_buffer,
                    DeviceConfig::default(),
                )
            });
            hid::register_device(device).await.unwrap();

            info!("Starting device task");
            loop {
                info!("Processing request");
                if let Err(e) = device.process_request().await {
                    error!("Device error: {:?}", e);
                }
            }
        }
    };
}

/// Define the host task with a compile-time command receive-buffer capacity.
///
/// The legacy form keeps its 128-byte allocation. Supply `buffer_size = N` for
/// larger framed host commands, independently of device response storage.
#[macro_export]
macro_rules! define_i2c_passthrough_host_task {
    ($bus:ty, $int_in:ty, $int_out:ty) => {
        $crate::define_i2c_passthrough_host_task!($bus, $int_in, $int_out, buffer_size = 128);
    };

    ($bus:ty, $int_in:ty, $int_out:ty, buffer_size = $buffer_size:expr $(,)?) => {
        pub async fn host_task(
            bus: $bus,
            int_signal: &'static $crate::i2c::passthrough::InterruptSignal<$int_in, $int_out>,
        ) {
            use ::embassy_sync::once_lock::OnceLock;
            use ::embedded_services::{comms, define_static_buffer, error, info};
            use $crate::i2c::{Host, HostConfig};

            info!("Creating HIDI2C Host");
            define_static_buffer!(host_buffer, u8, [0; $buffer_size]);
            static HOST: OnceLock<Host<$bus>> = OnceLock::new();
            let host = HOST.get_or_init(|| {
                Host::new(
                    HID_ID0,
                    bus,
                    host_buffer::get_mut().unwrap(),
                    HostConfig::default(),
                )
            });
            comms::register_endpoint(host, &host.tp).await.unwrap();

            loop {
                info!("Host Processing");
                let res = host.wait_request().await;
                if let Err(e) = res {
                    int_signal.reset().await;
                    error!("Host error {:?}", e);
                    continue;
                }

                // Deassert the interrupt signal
                // This should happen before we finish processing the request
                // to avoid triggering a spurious interrupt to the host
                int_signal.deassert().await;

                let access = res.unwrap();
                if let Err(e) = host.process_request(access).await {
                    error!("Host error {:?}", e);
                    int_signal.reset().await;
                    continue;
                }

                if let Err(e) = host.send_response().await {
                    error!("Host error {:?}", e);
                    int_signal.reset().await;
                    continue;
                }

                // Allow interrupts from the device again
                int_signal.release().await;
            }
        }
    };
}

#[macro_export]
macro_rules! define_i2c_passthrough_interrupt_task {
    ($int_in:ty, $int_out:ty) => {
        pub async fn interrupt_task(int_signal: &'static $crate::i2c::passthrough::InterruptSignal<$int_in, $int_out>) {
            ::embedded_services::info!("Starting interrupt task");
            loop {
                // Preserve the existing pin-error policy without introducing
                // an unbounded log stream. Request failures are handled by the
                // host task; GPIO-error retry policy is a separate concern.
                let _ = int_signal.process().await;
            }
        }
    };
}

/// Define the bridge tasks, optionally selecting independent buffer capacities.
///
/// Append `device_buffer_size = N`, `host_buffer_size = M`, or both in that order
/// to select response and command storage. Defaults remain 512 and 128 bytes.
#[macro_export]
macro_rules! define_i2c_passthrough_task {
    ($host_bus:ty, $device_bus:ty, $int_in:ty, $int_out:ty $(, device_buffer_size = $device_buffer_size:expr)? $(, host_buffer_size = $host_buffer_size:expr)?) => {
        mod task {
            use super::*;
            // Spawn these 3 tasks using the executor of your choice
            $crate::define_i2c_passthrough_device_task!($device_bus $(, buffer_size = $device_buffer_size)?);
            $crate::define_i2c_passthrough_host_task!($host_bus, $int_in, $int_out $(, buffer_size = $host_buffer_size)?);
            $crate::define_i2c_passthrough_interrupt_task!($int_in, $int_out);
        }
    };
}

#[cfg(test)]
pub(crate) mod tests {
    // Compile both composite forms, including the host/interrupt task bodies.
    // Pending pin/bus implementations are type fixtures, not hardware models;
    // device-task allocation and bridge behavior are exercised in device.rs.
    #![allow(dead_code)]

    use core::convert::Infallible;
    use core::future::pending;

    use embedded_hal::digital::{ErrorType as DigitalErrorType, OutputPin};
    use embedded_hal::i2c::{ErrorKind, ErrorType};
    use embedded_hal_async::digital::Wait;
    use embedded_hal_async::i2c::{I2c, Operation};

    use crate::i2c::{Command, I2cSlaveAsync};

    const HID_ID0: embedded_services::hid::DeviceId = embedded_services::hid::DeviceId(0);

    pub struct Bus;

    impl ErrorType for Bus {
        type Error = ErrorKind;
    }

    impl I2c for Bus {
        async fn transaction(&mut self, _: u8, _: &mut [Operation<'_>]) -> Result<(), Self::Error> {
            pending().await
        }
    }

    pub struct Slave;

    impl I2cSlaveAsync for Slave {
        type Error = ErrorKind;

        async fn listen(&mut self) -> Result<Command, Self::Error> {
            pending().await
        }

        async fn respond_to_write(&mut self, _: &mut [u8]) -> Result<(), Self::Error> {
            pending().await
        }

        async fn respond_to_read(&mut self, _: &[u8]) -> Result<(), Self::Error> {
            pending().await
        }
    }

    pub struct Pin;

    impl DigitalErrorType for Pin {
        type Error = Infallible;
    }

    impl OutputPin for Pin {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl Wait for Pin {
        async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
            pending().await
        }

        async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
            pending().await
        }

        async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
            pending().await
        }

        async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
            pending().await
        }

        async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
            pending().await
        }
    }

    mod legacy {
        use super::*;

        crate::define_i2c_passthrough_task!(Slave, Bus, Pin, Pin);
    }

    mod configured {
        use super::*;

        crate::define_i2c_passthrough_task!(Slave, Bus, Pin, Pin, device_buffer_size = 1129);
    }

    mod host_configured {
        use super::*;

        crate::define_i2c_passthrough_task!(Slave, Bus, Pin, Pin, host_buffer_size = 131);
    }

    mod independently_configured {
        use super::*;

        crate::define_i2c_passthrough_task!(Slave, Bus, Pin, Pin, device_buffer_size = 1129, host_buffer_size = 131);
    }
}
