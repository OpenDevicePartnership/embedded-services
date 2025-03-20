#![no_std]

use embassy_futures::select::select;
use embassy_futures::select::Either::{First, Second};
use embedded_batteries_async::charger::{MilliAmps, MilliVolts};
use embedded_services::comms::{self, External};
use embedded_services::ec_type::message::BatteryMessage;

mod charger;
mod fuel_gauge;

/// Example OEM messages.
/// True OEM messages should exist in OEM service, this is just an example.
#[derive(Copy, Clone, Debug)]
pub enum OemMessage {
    ChargeVoltage(MilliVolts),
    ChargeCurrent(MilliAmps),
}

/// Generic to hold OEM messages and standard ACPI messages
/// Can add more as more services have messages
#[derive(Copy, Clone, Debug)]
pub enum BatteryMsgs {
    Acpi(BatteryMessage),
    Oem(OemMessage),
}

/// Battery Service Errors
#[derive(Copy, Clone, Debug)]
pub enum BatteryServiceErrors {
    BufferFull,
    ChargerBusError,
    FuelGaugeBusError,
}

pub struct Service<
    SmartCharger: embedded_batteries_async::charger::Charger,
    SmartBattery: embedded_batteries_async::smart_battery::SmartBattery,
> {
    pub endpoint: comms::Endpoint,
    pub charger: charger::Charger<SmartCharger>,
    pub fuel_gauge: fuel_gauge::FuelGauge<SmartBattery>,
}

impl<
        SmartCharger: embedded_batteries_async::charger::Charger,
        SmartBattery: embedded_batteries_async::smart_battery::SmartBattery,
    > Service<SmartCharger, SmartBattery>
{
    pub fn new(smart_charger: SmartCharger, fuel_gauge: SmartBattery) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Battery)),
            charger: charger::Charger::new(smart_charger),
            fuel_gauge: fuel_gauge::FuelGauge::new(fuel_gauge),
        }
    }

    /// Function to request a variable amount of battery messages.
    /// Intended to be called within a timer callback context, for example to get the voltage every 1 second.
    pub async fn broadcast_dynamic_battery_msgs(&self, messages: &[BatteryMsgs]) {
        for msg in messages {
            match msg {
                BatteryMsgs::Acpi(BatteryMessage::CycleCount(_)) => self.fuel_gauge.rx.send(*msg).await,

                // BST
                BatteryMsgs::Acpi(BatteryMessage::State(_)) => self.fuel_gauge.rx.send(*msg).await,
                BatteryMsgs::Acpi(BatteryMessage::PresentRate(_)) => self.fuel_gauge.rx.send(*msg).await,
                BatteryMsgs::Acpi(BatteryMessage::RemainCap(_)) => self.fuel_gauge.rx.send(*msg).await,
                BatteryMsgs::Acpi(BatteryMessage::PresentVolt(_)) => self.fuel_gauge.rx.send(*msg).await,

                // More message support to be added.
                _ => todo!(),
            }
        }
    }

    fn handle_transport_msg(&self, msg: BatteryMsgs) -> Result<(), BatteryServiceErrors> {
        match msg {
            BatteryMsgs::Acpi(msg) => match msg {
                // Route to charger buffer or fuel gauge buffer
                _ => todo!(),
            },
            BatteryMsgs::Oem(msg) => match msg {
                // Route to charger buffer or fuel gauge buffer
                OemMessage::ChargeVoltage(_) => self
                    .charger
                    .rx
                    .try_send(BatteryMsgs::Oem(msg))
                    .map_err(|_| BatteryServiceErrors::BufferFull),
                _ => todo!(),
            },
        }
    }

    /// Function that handles responses from the charger and fuel gauge and sends messages over the endpoint.
    pub async fn handle_charger_fuel_gauge_msg(&self) -> Result<(), BatteryServiceErrors> {
        let charger_fut = self.charger.tx.receive();
        let fuel_gauge_fut = self.fuel_gauge.tx.receive();

        let msg = match select(charger_fut, fuel_gauge_fut).await {
            First(res) => match res {
                Ok(msg) => msg,
                Err(e) => match e {
                    charger::ChargerError::Bus => return Err(BatteryServiceErrors::ChargerBusError),
                },
            },
            Second(res) => match res {
                Ok(msg) => msg,
                Err(e) => match e {
                    fuel_gauge::FuelGaugeError::Bus => return Err(BatteryServiceErrors::FuelGaugeBusError),
                },
            },
        };

        // Route the message over the comms service or the charger or fuel gauge.
        // Individual messages can be handled in this match statement as exceptions.
        match msg {
            BatteryMsgs::Acpi(msg) => {
                self.endpoint
                    .send(comms::EndpointID::External(External::Host), &msg)
                    .await
                    .unwrap();
            }
            _ => todo!(),
        }
        Ok(())
    }
}

impl<
        SmartCharger: embedded_batteries_async::charger::Charger,
        SmartBattery: embedded_batteries_async::smart_battery::SmartBattery,
    > comms::MailboxDelegate for Service<SmartCharger, SmartBattery>
{
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(msg) = message.data.get::<BatteryMessage>() {
            self.handle_transport_msg(BatteryMsgs::Acpi(*msg))
                .map_err(|e| match e {
                    BatteryServiceErrors::BufferFull => comms::MailboxDelegateError::BufferFull,
                    _ => comms::MailboxDelegateError::Other,
                })?;
        } else if let Some(msg) = message.data.get::<OemMessage>() {
            self.handle_transport_msg(BatteryMsgs::Oem(*msg)).map_err(|e| match e {
                BatteryServiceErrors::BufferFull => comms::MailboxDelegateError::BufferFull,
                _ => comms::MailboxDelegateError::Other,
            })?;
        } else {
            return Err(comms::MailboxDelegateError::MessageNotFound);
        }

        Ok(())
    }
}

/// Generates the service instance and
///
/// - battery_service_init()
/// - battery_service_task()
/// - charger_task()
/// - fuel_gauge_task()
#[macro_export]
macro_rules! create_battery_service {
    ($charger:ident, $charger_bus:path, $fuel_gauge:ident, $fuel_gauge_bus:path) => {
        use ::battery_service::{BatteryServiceErrors, Service};
        use ::embedded_services::{comms, error};
        static SERVICE: OnceLock<Service<$charger<$charger_bus>, $fuel_gauge<$fuel_gauge_bus>>> = OnceLock::new();

        pub async fn battery_service_init(chg_bus: $charger_bus, fg_bus: $fuel_gauge_bus) {
            let battery_service =
                SERVICE.get_or_init(|| Service::new($charger::new(chg_bus), $fuel_gauge::new(fg_bus)));

            comms::register_endpoint(battery_service, &battery_service.endpoint)
                .await
                .unwrap();
        }

        // Tasks
        #[embassy_executor::task]
        async fn battery_service_task(spawner: Spawner) {
            // Block until service is initialized
            let s = SERVICE.get().await;

            spawner.must_spawn(charger_task());
            spawner.must_spawn(fuel_gauge_task());

            loop {
                if let Err(e) = s.handle_charger_fuel_gauge_msg().await {
                    match e {
                        BatteryServiceErrors::ChargerBusError => error!("Charger bus error"),
                        BatteryServiceErrors::FuelGaugeBusError => error!("FG bus error"),
                        BatteryServiceErrors::BufferFull => error!("Buffer full"),
                    }
                }
            }
        }

        #[embassy_executor::task]
        async fn charger_task() {
            // Block until service is initialized
            let s = SERVICE.get().await;

            loop {
                s.charger.process_service_message().await;
            }
        }

        #[embassy_executor::task]
        async fn fuel_gauge_task() {
            // Block until service is initialized
            let s = SERVICE.get().await;

            loop {
                s.fuel_gauge.process_service_message().await;
            }
        }
    };
}
