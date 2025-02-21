use core::cell::RefCell;

use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};

use crate::BatteryMsgs;

/// Tasks breakdown:
/// Task to recv messages from battery_service (rx_msg_from_service())

pub enum FuelGaugeError {
    Bus,
}

pub struct FuelGauge<F: embedded_batteries_async::smart_battery::SmartBattery> {
    device: RefCell<F>,
    pub(crate) rx: Channel<NoopRawMutex, crate::BatteryMsgs, 1>,

    // Should size of channel be increased as a flurry of messages will need to be sent with broadcasts?
    pub(crate) tx: Channel<NoopRawMutex, Result<crate::BatteryMsgs, FuelGaugeError>, 1>,
}

impl<F: embedded_batteries_async::smart_battery::SmartBattery> FuelGauge<F> {
    pub fn new(fuel_gauge: F) -> Self {
        FuelGauge {
            device: RefCell::new(fuel_gauge),
            rx: Channel::new(),
            tx: Channel::new(),
        }
    }

    pub async fn rx_msg_from_service(&self) {
        let rx_msg = self.rx.receive().await;
        embedded_services::info!("Recv'd FG message!");
        match rx_msg {
            BatteryMsgs::Acpi(msg) => match msg {
                crate::ESpiMessage::BatCycleCount(_) => {
                    let res = self
                        .device
                        .borrow_mut()
                        .cycle_count()
                        .await
                        .map(|cycles| BatteryMsgs::Acpi(crate::ESpiMessage::BatCycleCount(cycles.into())))
                        .map_err(|_| FuelGaugeError::Bus);
                    self.tx.send(res).await;
                }
                _ => todo!(),
            },
            BatteryMsgs::Oem(msg) => match msg {
                _ => todo!(),
            },
        }
    }
}
