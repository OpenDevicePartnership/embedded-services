use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embedded_batteries_async::smart_battery::CapacityModeValue;
use embedded_services::{error, info};

use crate::BatteryMsgs;

#[derive(Clone, Copy, Debug)]
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

    pub async fn process_service_message(&self) {
        let rx_msg = self.rx.receive().await;
        match rx_msg {
            BatteryMsgs::Acpi(msg) => match msg {
                crate::BatteryMessage::CycleCount(_) => {
                    let res = self
                        .device
                        .borrow_mut()
                        .cycle_count()
                        .await
                        .map(|cycles| BatteryMsgs::Acpi(crate::BatteryMessage::CycleCount(cycles.into())))
                        .map_err(|_| FuelGaugeError::Bus);
                    self.tx.send(res).await;
                }

                // BST
                crate::BatteryMessage::State(_) => {
                    let res = self
                        .device
                        .borrow_mut()
                        .battery_status()
                        .await
                        .map(|f| {
                            // TODO: Add bit 2 and 3
                            BatteryMsgs::Acpi(crate::BatteryMessage::State(if f.discharging() { 0x01 } else { 0x02 }))
                        })
                        .map_err(|_| FuelGaugeError::Bus);
                    if res.is_ok() {
                        info!(
                            "State = {}",
                            if let BatteryMsgs::Acpi(crate::BatteryMessage::State(state)) = res.unwrap() {
                                state
                            } else {
                                unreachable!();
                            }
                        )
                    }
                    self.tx.send(res).await;
                }
                crate::BatteryMessage::PresentRate(_) => {
                    let res = self
                        .device
                        .borrow_mut()
                        .current()
                        .await
                        .map(|f| BatteryMsgs::Acpi(crate::BatteryMessage::PresentRate(f.unsigned_abs().into())))
                        .map_err(|_| FuelGaugeError::Bus);
                    if res.is_ok() {
                        info!(
                            "Present Rate = {}",
                            if let BatteryMsgs::Acpi(crate::BatteryMessage::PresentRate(current)) = res.unwrap() {
                                current
                            } else {
                                unreachable!();
                            }
                        )
                    }
                    self.tx.send(res).await;
                }
                crate::BatteryMessage::RemainCap(_) => {
                    let res = self
                        .device
                        .borrow_mut()
                        .remaining_capacity()
                        .await
                        .map(|f| {
                            BatteryMsgs::Acpi(crate::BatteryMessage::RemainCap(match f {
                                CapacityModeValue::MilliAmpUnsigned(cap)
                                | CapacityModeValue::CentiWattUnsigned(cap) => cap.into(),
                            }))
                        })
                        .map_err(|_| FuelGaugeError::Bus);
                    if res.is_ok() {
                        info!(
                            "Remaining cap = {}",
                            if let BatteryMsgs::Acpi(crate::BatteryMessage::RemainCap(cap)) = res.unwrap() {
                                cap
                            } else {
                                unreachable!();
                            }
                        )
                    }
                    self.tx.send(res).await;
                }
                crate::BatteryMessage::PresentVolt(_) => {
                    let res = self
                        .device
                        .borrow_mut()
                        .voltage()
                        .await
                        .map(|f| BatteryMsgs::Acpi(crate::BatteryMessage::PresentVolt(f.into())))
                        .map_err(|_| FuelGaugeError::Bus);
                    if res.is_ok() {
                        info!(
                            "Present voltage = {}",
                            if let BatteryMsgs::Acpi(crate::BatteryMessage::PresentVolt(v)) = res.unwrap() {
                                v
                            } else {
                                unreachable!();
                            }
                        )
                    }
                    self.tx.send(res).await;
                }
                _ => error!("Unexpected message sent to charger"),
            },
            BatteryMsgs::Oem(msg) => match msg {
                _ => error!("Unexpected message sent to charger"),
            },
        }
    }
}
