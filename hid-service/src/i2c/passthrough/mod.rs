mod interrupt;

pub use interrupt::*;

#[macro_export]
macro_rules! define_i2c_passthrough_device_task {
    ($bus:ty) => {
        #[::embassy_executor::task]
        async fn device_task(bus: $bus, id: ::embedded_services::hid::DeviceId, addr: u8) {
            use ::embassy_sync::once_lock::OnceLock;
            use ::embedded_services::{define_static_buffer, error, hid, info};
            use $crate::i2c::Device;
            define_static_buffer!(gen_buffer, u8, [0; 512]);
            let gen_buffer = gen_buffer::get_mut().unwrap();

            let custom_regs = RegisterFile {
                hid_desc_reg: 0x00,
                report_desc_reg: 0x20,
                input_reg: 0x60,
                output_reg: 0x62,
                command_reg: 0x64,
                data_reg: 0x66,
            };

            info!("Create HID passthrough device {}", id.0);
            static DEVICE: OnceLock<Device<u8, $bus>> = OnceLock::new();
            let device = DEVICE.get_or_init(|| Device::new(id, addr, bus, custom_regs, gen_buffer));
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

#[macro_export]
macro_rules! define_i2c_passthrough_host_task {
    ($bus:ty, $int_in:ty, $int_out:ty) => {
        #[::embassy_executor::task]
        async fn host_task(
            bus: $bus,
            int_signal: &'static $crate::i2c::passthrough::InterruptSignal<$int_in, $int_out>,
        ) {
            use ::embassy_sync::once_lock::OnceLock;
            use ::embedded_services::{comms, define_static_buffer, error, info};
            use $crate::i2c::Host;

            info!("Creating HIDI2C Host");
            define_static_buffer!(host_buffer, u8, [0; 128]);
            static HOST: OnceLock<Host<$bus>> = OnceLock::new();
            let host = HOST.get_or_init(|| Host::new(HID_ID0, bus, host_buffer::get_mut().unwrap()));
            comms::register_endpoint(host, &host.tp).await.unwrap();

            loop {
                info!("Host Processing");
                let res = host.wait_request().await;
                if let Err(e) = res {
                    int_signal.reset();
                    error!("Host error {:?}", e);
                    continue;
                }

                // Deassert the interrupt signal
                // This should happen before we finish processing the request
                // to avoid triggering a spurious interrupt to the host
                int_signal.deassert();

                let access = res.unwrap();
                if let Err(e) = host.process_request(access).await {
                    error!("Host error {:?}", e);
                    int_signal.reset();
                    continue;
                }

                if let Err(e) = host.send_response().await {
                    error!("Host error {:?}", e);
                    int_signal.reset();
                    continue;
                }

                // Allow interrupts from the device again
                int_signal.release();
            }
        }
    };
}

#[macro_export]
macro_rules! define_i2c_passthrough_interrupt_task {
    ($int_in:ty, $int_out:ty) => {
        #[::embassy_executor::task]
        async fn interrupt_task(int_signal: &'static $crate::i2c::passthrough::InterruptSignal<$int_in, $int_out>) {
            ::embedded_services::info!("Starting interrupt task");
            loop {
                int_signal.process().await;
            }
        }
    };
}

#[macro_export]
macro_rules! define_i2c_passthrough_task {
    ($name:ident, $host_bus:ty, $device_bus:ty, $int_in:ty, $int_out:ty) => {
        mod $name {
            use $crate::{
                define_i2c_passthrough_device_task, define_i2c_passthrough_host_task,
                define_i2c_passthrough_interrupt_task,
            };

            use super::*;

            define_i2c_passthrough_device_task!($device_bus);
            define_i2c_passthrough_host_task!($host_bus, $int_in, $int_out);
            define_i2c_passthrough_interrupt_task!($int_in, $int_out);

            pub fn spawn(
                spawner: ::embassy_executor::Spawner,
                host_bus: $host_bus,
                device_bus: $device_bus,
                device_id: ::embedded_services::hid::DeviceId,
                device_addr: u8,
                int_in: $int_in,
                int_out: $int_out,
            ) {
                use ::embassy_sync::once_lock::OnceLock;
                use $crate::i2c::passthrough::InterruptSignal;

                static INT_SIGNAL: OnceLock<InterruptSignal<$int_in, $int_out>> = OnceLock::new();
                let int_signal = INT_SIGNAL.get_or_init(|| InterruptSignal::new(int_in, int_out));

                spawner.must_spawn(device_task(device_bus, device_id, device_addr));
                spawner.must_spawn(host_task(host_bus, int_signal));
                spawner.must_spawn(interrupt_task(int_signal));
            }
        }
    };
}
