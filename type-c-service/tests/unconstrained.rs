//! Test the unconstrained power logic of the type-C service
#![allow(dead_code)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

use embassy_time::{Duration, Timer, with_timeout};
use embedded_services::named::Named;
use embedded_usb_pd::{PowerRole, pdo, type_c::ConnectionState};
use log::info;
use power_policy_interface::{
    capability::PowerCapability,
    service::{UnconstrainedState, event::Event as PowerPolicyEvent},
};
use type_c_interface::{
    control::pd::{PdSinkInfo, PortStatus, SinkContract},
    port::event::{PortEvent, PortStatusEventBitfield},
};
use type_c_interface_test_mocks::controller::{FnCall as ControllerFnCall, pd::FnCall as PdFnCall};
use type_c_service::controller::{
    config::{Config as PortConfig, UnconstrainedSink},
    event::Event,
};

use crate::common::{DEFAULT_PER_CALL_TIMEOUT, PowerPolicyServiceReceiver, Test, TestPort, TypeCServiceReceiver};

mod common;

/// The scenarios run for several seconds because the power policy service settles for 800ms after
/// every consumer connect.
const TEST_DURATION: Duration = Duration::from_secs(30);

/// Time to wait for the power policy service to broadcast an unconstrained state change.
const EVENT_TIMEOUT: Duration = Duration::from_secs(3);

/// Polling interval used while waiting for the type-C service to reach the controller mocks.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Number of results queued on each mock per step, enough to cover the most call-heavy step.
const PRIME_DEPTH: usize = 4;

/// Power threshold that [`CAPABILITY`] exceeds.
const UNCONSTRAINED_THRESHOLD_MW: u32 = 15000;

/// Sink contract offered by every simulated port partner.
const CAPABILITY: PowerCapability = PowerCapability {
    voltage_mv: 20000,
    current_ma: 5000,
};

/// Port status of a detached port.
const DETACHED: PortStatus = PortStatus::new();

/// Port status of an attached sink offering [`CAPABILITY`].
fn sink_status(unconstrained: bool) -> PortStatus {
    let pdo = pdo::sink::Pdo::Fixed(pdo::sink::FixedData {
        voltage_mv: CAPABILITY.voltage_mv,
        operational_current_ma: CAPABILITY.current_ma,
        ..Default::default()
    });
    PortStatus {
        available_sink_contract: Some(SinkContract {
            capability: CAPABILITY,
            pd: pdo::Rdo::for_pdo(0, pdo).map(|rdo| PdSinkInfo {
                rx_fixed_5v_data: pdo::source::FixedData {
                    voltage_mv: CAPABILITY.voltage_mv,
                    current_ma: CAPABILITY.current_ma,
                    unconstrained_power: unconstrained,
                    ..Default::default()
                },
                pdo,
                rdo,
            }),
        }),
        connection_state: Some(ConnectionState::Attached),
        power_role: PowerRole::Sink,
        ..PortStatus::new()
    }
}

/// Value of the most recent `set_unconstrained_power` call recorded by the port's controller mock.
async fn last_unconstrained_call(port: &TestPort<'_, '_>) -> Option<bool> {
    port.mock
        .lock()
        .await
        .fn_calls
        .iter()
        .rev()
        .find_map(|call| match call {
            ControllerFnCall::Pd(PdFnCall::SetUnconstrainedPower(_, unconstrained)) => Some(*unconstrained),
            _ => None,
        })
}

/// Driver for multi-step unconstrained scenarios.
///
/// Every step re-primes all three mocks with the status their port currently reports, because the
/// type-C service reads the port status a variable number of times depending on which branch of
/// the unconstrained logic runs.
struct Scenario<'a, 'port, 'ch> {
    ports: [&'a TestPort<'port, 'ch>; 3],
    power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
    statuses: [PortStatus; 3],
}

impl<'a, 'port, 'ch> Scenario<'a, 'port, 'ch> {
    fn new(
        ports: [&'a TestPort<'port, 'ch>; 3],
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
    ) -> Self {
        Self {
            ports,
            power_policy_receiver,
            statuses: [DETACHED; 3],
        }
    }

    /// Drop the calls recorded by the previous step and queue results for the next one.
    async fn prime(&self) {
        for (port, status) in self.ports.iter().zip(self.statuses) {
            let mut mock = port.mock.lock().await;
            mock.fn_calls.clear();
            mock.next_result_get_port_status.clear();
            mock.next_result_enable_sink_path.clear();
            mock.next_result_set_unconstrained_power.clear();

            for _ in 0..PRIME_DEPTH {
                mock.next_result_get_port_status.push_back(Ok(status));
                mock.next_result_enable_sink_path.push_back(Ok(()));
                mock.next_result_set_unconstrained_power.push_back(Ok(()));
            }
        }
    }

    async fn process(&self, index: usize, status_event: PortStatusEventBitfield) {
        self.ports[index]
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(status_event)))
            .await
            .unwrap();
    }

    /// Simulate a sink attaching to the given port.
    async fn connect_sink(&mut self, index: usize, unconstrained: bool) {
        info!("Connecting port {index}, unconstrained: {unconstrained}");
        self.statuses[index] = sink_status(unconstrained);
        self.prime().await;

        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);
        status_event.set_new_power_contract_as_consumer(true);
        // Report the sink as ready so the port doesn't arm its software sink-ready timer.
        status_event.set_sink_ready(true);
        self.process(index, status_event).await;
    }

    /// Simulate a detach on the given port.
    async fn detach(&mut self, index: usize) {
        info!("Disconnecting port {index}");
        self.statuses[index] = DETACHED;
        self.prime().await;

        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);
        self.process(index, status_event).await;
    }

    /// Wait for the power policy service to broadcast the expected unconstrained state, then check
    /// the flag the type-C service applied to each port.
    async fn expect_unconstrained(&self, expected: UnconstrainedState, expected_flags: [bool; 3]) {
        let state = with_timeout(EVENT_TIMEOUT, async {
            loop {
                if let PowerPolicyEvent::Unconstrained(state) = self.power_policy_receiver.receive().await {
                    return state;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("Did not receive unconstrained event"));
        assert_eq!(state, expected);

        for (port, expected) in self.ports.iter().zip(expected_flags) {
            let name = port.mock.lock().await.name();
            let actual = with_timeout(DEFAULT_PER_CALL_TIMEOUT, async {
                loop {
                    if let Some(unconstrained) = last_unconstrained_call(port).await {
                        return unconstrained;
                    }
                    Timer::after(POLL_INTERVAL).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("({name}): set_unconstrained_power was not called"));
            assert_eq!(actual, expected, "({name}): unexpected unconstrained flag");
        }
    }

    /// Assert that the unconstrained state doesn't change and that no port is reconfigured.
    async fn expect_no_unconstrained_change(&self) {
        // Drain the events this step legitimately produces until the services go idle.
        while let Ok(event) = with_timeout(DEFAULT_PER_CALL_TIMEOUT, self.power_policy_receiver.receive()).await {
            assert!(
                !matches!(event, PowerPolicyEvent::Unconstrained(_)),
                "unexpected unconstrained event"
            );
        }

        for port in self.ports {
            let name = port.mock.lock().await.name();
            assert!(
                last_unconstrained_call(port).await.is_none(),
                "({name}): unexpected set_unconstrained_power call"
            );
        }
    }
}

/// Test the unconstrained logic driven by the unconstrained bit reported by the port partner.
struct TestUnconstrained;

impl Test for TestUnconstrained {
    async fn run<'port, 'ch>(
        &mut self,
        _service: &common::TypeCServiceMutexType<'port, 'ch>,
        _type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        port1: TestPort<'port, 'ch>,
        port2: TestPort<'port, 'ch>,
    ) {
        let mut scenario = Scenario::new([&port0, &port1, &port2], power_policy_receiver);

        // A single unconstrained port unconstrains the other ports, but not itself: the system
        // stops being unconstrained as soon as that port starts sourcing.
        scenario.connect_sink(0, true).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(true, 1), [false, true, true])
            .await;

        // A constrained sink leaves the unconstrained state untouched.
        scenario.connect_sink(1, false).await;
        scenario.expect_no_unconstrained_change().await;

        // Losing the only unconstrained port constrains every port.
        scenario.detach(0).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(false, 0), [false, false, false])
            .await;

        // Detaching the remaining constrained port changes nothing.
        scenario.detach(1).await;
        scenario.expect_no_unconstrained_change().await;

        scenario.connect_sink(0, true).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(true, 1), [false, true, true])
            .await;

        // With more than one unconstrained port available the system stays unconstrained even if
        // one of them starts sourcing, so every port gets the flag.
        scenario.connect_sink(1, true).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(true, 2), [true, true, true])
            .await;

        scenario.connect_sink(2, true).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(true, 3), [true, true, true])
            .await;

        // Still more than one unconstrained port after the current consumer goes away.
        scenario.detach(0).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(true, 2), [true, true, true])
            .await;

        // Back down to a single unconstrained port, which is the current consumer.
        scenario.detach(2).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(true, 1), [true, false, true])
            .await;

        // Nothing left, every port ends up constrained.
        scenario.detach(1).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(false, 0), [false, false, false])
            .await;
    }
}

/// Test the unconstrained logic for a sink that doesn't report the unconstrained bit.
///
/// [`UnconstrainedSink::PowerThresholdMilliwatts`] makes the port report unconstrained power based
/// on contract power alone, so the service must attribute the system unconstrained state to that
/// port and hold it constrained, just like a partner-reported unconstrained sink.
struct TestUnconstrainedPowerThreshold;

impl Test for TestUnconstrainedPowerThreshold {
    async fn run<'port, 'ch>(
        &mut self,
        _service: &common::TypeCServiceMutexType<'port, 'ch>,
        _type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        port1: TestPort<'port, 'ch>,
        port2: TestPort<'port, 'ch>,
    ) {
        let mut scenario = Scenario::new([&port0, &port1, &port2], power_policy_receiver);

        scenario.connect_sink(0, false).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(true, 1), [false, true, true])
            .await;

        scenario.detach(0).await;
        scenario
            .expect_unconstrained(UnconstrainedState::new(false, 0), [false, false, false])
            .await;
    }
}

#[tokio::test]
async fn unconstrained() {
    common::run_test(TEST_DURATION, Default::default(), Default::default(), TestUnconstrained).await;
}

#[tokio::test]
async fn unconstrained_power_threshold() {
    let mut port_config = [PortConfig::default(); 3];
    port_config[0].unconstrained_sink = UnconstrainedSink::PowerThresholdMilliwatts(UNCONSTRAINED_THRESHOLD_MW);

    common::run_test(
        TEST_DURATION,
        Default::default(),
        port_config,
        TestUnconstrainedPowerThreshold,
    )
    .await;
}
