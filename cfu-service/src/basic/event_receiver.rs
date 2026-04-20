use embassy_futures::select::{Either, select};
use embassy_time::{Instant, Timer};
use embedded_services::{debug, error, sync::Lockable};

use crate::basic::{
    Output,
    config::EventReceiver as Config,
    state::{FwUpdateState, SharedState},
};

/// CFU events
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event {
    /// CFU request
    Request(crate::component::RequestData),
    /// Recovery tick
    ///
    /// Occurs when the FW update has timed out to abort the update and return hardware to its normal state
    RecoveryTick,
}

/// Struct to receive CFU events.
pub struct EventReceiver<'a, Shared: Lockable<Inner = SharedState>> {
    /// Config
    config: Config,
    /// CFU device used for firmware updates
    cfu_device: &'static crate::component::CfuDevice,
    /// State shared with [`crate::basic::Updater`]
    shared_state: &'a Shared,
}

impl<'a, Shared: Lockable<Inner = SharedState>> EventReceiver<'a, Shared> {
    /// Create a new CFU event receiver
    pub fn new(cfu_device: &'static crate::component::CfuDevice, shared_state: &'a Shared, config: Config) -> Self {
        Self {
            cfu_device,
            shared_state,
            config,
        }
    }

    /// Wait for the next CFU event
    pub async fn wait_next(&mut self) -> Event {
        loop {
            let (fw_update_state, next_recovery_tick) = {
                let state = self.shared_state.lock().await;
                (state.fw_update_state, state.next_recovery_tick)
            };
            match fw_update_state {
                FwUpdateState::Idle => {
                    // No FW update in progress, just wait for a command
                    return Event::Request(self.cfu_device.wait_request().await);
                }
                FwUpdateState::InProgress(ticks) => {
                    match select(self.cfu_device.wait_request(), Timer::at(next_recovery_tick)).await {
                        Either::First(command) => return Event::Request(command),
                        Either::Second(_) => {
                            debug!("CFU tick: {}", ticks);

                            let mut shared_state = self.shared_state.lock().await;
                            shared_state.next_recovery_tick = Instant::now() + self.config.recovery.tick_interval;

                            if ticks + 1 < self.config.recovery.update_timeout_ticks {
                                shared_state.fw_update_state = FwUpdateState::InProgress(ticks + 1);
                                continue;
                            } else {
                                error!(
                                    "FW update timed out after {} ticks",
                                    self.config.recovery.update_timeout_ticks
                                );
                                shared_state.fw_update_state = FwUpdateState::Recovery;
                                return Event::RecoveryTick;
                            }
                        }
                    }
                }
                FwUpdateState::Recovery => {
                    // Recovery state, wait for the next attempt to recover the device
                    let next_recovery_tick = self.shared_state.lock().await.next_recovery_tick;
                    Timer::at(next_recovery_tick).await;
                    self.shared_state.lock().await.next_recovery_tick =
                        Instant::now() + self.config.recovery.tick_interval;
                    debug!("FW update ticker ticked");
                    return Event::RecoveryTick;
                }
            }
        }
    }

    /// Finalize the processing of an output
    // TODO: remove this when we refactor CFU
    pub async fn finalize(&mut self, output: Output) {
        if let Output::CfuResponse(response) = output {
            self.cfu_device.send_response(response).await
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{basic::config::Recovery, component::CfuDevice};

    use super::*;
    use embassy_sync::mutex::Mutex;
    use embassy_time::{Duration, Instant, TimeoutError, with_timeout};
    use embedded_services::GlobalRawMutex;
    use static_cell::StaticCell;

    /// Test that we get recovery ticks as expected
    #[tokio::test]
    async fn test_recovery_timeout() {
        static CFU_DEVICE: StaticCell<CfuDevice> = StaticCell::new();

        // Maximum timeout for the recovery entry, actual time should be 1000, but gives us some margin
        const RECOVERY_ENTRY_MAX_TIMEOUT: Duration = Duration::from_millis(1100);
        // Maximum timeout for an individual recovery tick, actual time should be 100, but gives us some margin
        const RECOVERY_TICK_MAX_TIMEOUT: Duration = Duration::from_millis(110);
        // Expected measured interval between recovery ticks, actual time should be 100, but undershoot slightly for some margin
        const EXPECTED_RECOVERY_TICK_INTERVAL: Duration = Duration::from_millis(90);

        let shared_state: Mutex<GlobalRawMutex, _> = Mutex::new(SharedState::default());
        let cfu_device = CFU_DEVICE.init(CfuDevice::new(0));
        let recovery_config = Recovery {
            tick_interval: Duration::from_millis(100),
            update_timeout_ticks: 10,
        };

        let mut event_receiver = EventReceiver::new(
            cfu_device,
            &shared_state,
            Config {
                recovery: recovery_config,
            },
        );

        // First test the recovery timer isn't active in the idle state
        assert_eq!(
            with_timeout(RECOVERY_ENTRY_MAX_TIMEOUT, event_receiver.wait_next()).await,
            Err(TimeoutError),
        );
        assert_eq!(
            event_receiver.shared_state.lock().await.fw_update_state,
            FwUpdateState::Idle
        );

        // Start the recovery ticker, normally the update struct handles this.
        shared_state
            .lock()
            .await
            .enter_in_progress(recovery_config.tick_interval);

        let start = Instant::now();
        assert_eq!(
            with_timeout(RECOVERY_ENTRY_MAX_TIMEOUT, event_receiver.wait_next()).await,
            Ok(Event::RecoveryTick),
        );
        let duration = Instant::now() - start;

        // Check that we waited approximately the correct amount of time
        assert!(duration.as_millis() >= 1000);
        assert_eq!(
            event_receiver.shared_state.lock().await.fw_update_state,
            FwUpdateState::Recovery
        );

        // Check the first recovery tick after the state transition
        let start = Instant::now();
        assert_eq!(
            with_timeout(RECOVERY_TICK_MAX_TIMEOUT, event_receiver.wait_next()).await,
            Ok(Event::RecoveryTick),
        );
        let duration = Instant::now() - start;

        // Check that we waited approximately the correct amount of time
        assert!(duration >= EXPECTED_RECOVERY_TICK_INTERVAL);
        assert_eq!(
            event_receiver.shared_state.lock().await.fw_update_state,
            FwUpdateState::Recovery
        );

        // Check subsequent recovery ticks
        let start = Instant::now();
        assert_eq!(
            with_timeout(RECOVERY_TICK_MAX_TIMEOUT, event_receiver.wait_next()).await,
            Ok(Event::RecoveryTick),
        );
        let duration = Instant::now() - start;

        // Check that we waited approximately the correct amount of time
        assert!(duration >= EXPECTED_RECOVERY_TICK_INTERVAL);
        assert_eq!(
            event_receiver.shared_state.lock().await.fw_update_state,
            FwUpdateState::Recovery
        );
    }
}
