#![allow(clippy::unwrap_used)]

use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll, Waker};
use std::sync::Barrier;

use embassy_futures::block_on;
use embedded_mcu_hal::time::Datetime;
use odp_service_common::runnable_service::ServiceRunner;

use crate::mock::{MockDatetimeClock, MockNvramStorage};
use crate::{AcpiTimerId, AlarmExpiredWakePolicy, AlarmTimerSeconds, Runner, Service, TimeAlarmService};

const SOURCES: [AcpiTimerId; 2] = [AcpiTimerId::AcPower, AcpiTimerId::DcPower];

fn with_service(test: impl FnOnce(Service<'_>, Runner<'_>)) {
    let mut clock = MockDatetimeClock::Paused {
        frozen_time: Datetime::from_unix_timestamp(1_700_000_000),
    };
    let mut tz = MockNvramStorage::new(0);
    let mut ac_expiry = MockNvramStorage::new(u32::MAX);
    let mut ac_policy = MockNvramStorage::new(u32::MAX);
    let mut dc_expiry = MockNvramStorage::new(u32::MAX);
    let mut dc_policy = MockNvramStorage::new(u32::MAX);
    let mut resources = Default::default();
    let (service, runner) = block_on(Service::new(
        &mut resources,
        &mut clock,
        &mut tz,
        &mut ac_expiry,
        &mut ac_policy,
        &mut dc_expiry,
        &mut dc_policy,
    ))
    .unwrap();
    test(service, runner);
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn poll_runner(mut runner: Pin<&mut impl Future>) {
    // Embassy timers yield once even for a zero-second deadline.
    assert!(poll_once(runner.as_mut()).is_pending());
    assert!(poll_once(runner).is_pending());
}

fn wake_level(service: Service<'_>) -> Poll<bool> {
    poll_once(pin!(service.wait_for_wake_signal()))
}

fn advance_clock(service: Service<'_>, seconds: u64) {
    let mut timestamp = service.get_real_time().unwrap();
    timestamp.datetime = Datetime::from_unix_timestamp(timestamp.datetime.unix_timestamp() + seconds);
    service.set_real_time(timestamp).unwrap();
}

fn deliver_deadline(service: Service<'_>, timer_id: AcpiTimerId, runner: Pin<&mut impl Future>) {
    // Deliver an elapsed deadline after advancing the controlled RTC, without a wall-clock sleep.
    service.inner.timers.get_timer(timer_id).timer_signal.signal(Some(0));
    poll_runner(runner);
}

fn latch_both(service: Service<'_>, mut runner: Pin<&mut impl Future>) {
    for source in SOURCES {
        service.set_power_source(source);
        service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
        poll_runner(runner.as_mut());
        assert!(service.get_wake_status(source).timer_triggered_wake());
    }
}

#[test]
fn initial_level_is_false_and_default_source_is_ac() {
    with_service(|service, runner| {
        assert_eq!(wake_level(service), Poll::Ready(false));
        assert_eq!(wake_level(service), Poll::Pending);
        service
            .set_timer_value(AcpiTimerId::AcPower, AlarmTimerSeconds(0))
            .unwrap();
        let mut runner = pin!(runner.run());
        poll_runner(runner.as_mut());
        assert_eq!(wake_level(service), Poll::Ready(true));
        assert_eq!(wake_level(service), Poll::Pending);
        assert!(service.get_wake_status(AcpiTimerId::AcPower).timer_triggered_wake());
    });
}

#[test]
fn either_source_can_be_selected_before_running() {
    for source in SOURCES {
        with_service(|service, runner| {
            service.set_power_source(source);
            for timer_id in SOURCES {
                service
                    .set_expired_timer_policy(timer_id, AlarmExpiredWakePolicy(10))
                    .unwrap();
            }
            service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
            assert_eq!(wake_level(service), Poll::Ready(false));
            let mut runner = pin!(runner.run());
            poll_runner(runner.as_mut());
            assert_eq!(wake_level(service), Poll::Ready(true));
            let status = service.get_wake_status(source);
            assert!(status.timer_expired());
            assert!(status.timer_triggered_wake());
            assert_eq!(service.get_timer_value(source).unwrap(), AlarmTimerSeconds::DISABLED);
            for timer_id in SOURCES {
                assert_eq!(
                    service.get_expired_timer_policy(timer_id),
                    AlarmExpiredWakePolicy::NEVER
                );
            }
        });
    }
}

#[test]
fn inactive_expiry_waits_for_matching_source() {
    for source in SOURCES {
        with_service(|service, runner| {
            service.set_power_source(source.get_other_timer_id());
            service
                .set_expired_timer_policy(source, AlarmExpiredWakePolicy::INSTANTLY)
                .unwrap();
            service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
            assert_eq!(wake_level(service), Poll::Ready(false));
            let mut runner = pin!(runner.run());
            poll_runner(runner.as_mut());
            assert!(service.get_wake_status(source).timer_expired());
            assert!(!service.get_wake_status(source).timer_triggered_wake());
            assert_eq!(wake_level(service), Poll::Pending);
            service.set_power_source(source);
            poll_runner(runner.as_mut());
            assert_eq!(wake_level(service), Poll::Ready(true));
        });
    }
}

#[test]
fn never_orphans_only_an_inactive_expiry() {
    for source in SOURCES {
        with_service(|service, runner| {
            service.set_power_source(source.get_other_timer_id());
            service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
            assert_eq!(wake_level(service), Poll::Ready(false));
            let mut runner = pin!(runner.run());
            poll_runner(runner.as_mut());
            assert!(service.get_wake_status(source).timer_expired());
            assert!(!service.get_wake_status(source).timer_triggered_wake());
            service.set_power_source(source);
            advance_clock(service, 100);
            poll_runner(runner.as_mut());
            assert_eq!(wake_level(service), Poll::Pending);
            service
                .set_expired_timer_policy(source, AlarmExpiredWakePolicy::INSTANTLY)
                .unwrap();
            poll_runner(runner.as_mut());
            assert_eq!(wake_level(service), Poll::Pending);

            service
                .set_expired_timer_policy(source, AlarmExpiredWakePolicy::NEVER)
                .unwrap();
            service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
            poll_runner(runner.as_mut());
            assert_eq!(wake_level(service), Poll::Ready(true));
        });
    }
}

#[test]
fn policy_delay_preserves_every_unpolled_source_transition() {
    for source in SOURCES {
        with_service(|service, runner| {
            let other = source.get_other_timer_id();
            service.set_power_source(other);
            service
                .set_expired_timer_policy(source, AlarmExpiredWakePolicy(10))
                .unwrap();
            service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
            assert_eq!(wake_level(service), Poll::Ready(false));
            let mut runner = pin!(runner.run());
            poll_runner(runner.as_mut());

            service.set_power_source(source);
            advance_clock(service, 3);
            service.set_power_source(other);
            advance_clock(service, 100);
            service.set_power_source(source);
            advance_clock(service, 2);
            service.set_power_source(other);
            advance_clock(service, 100);
            service.set_power_source(source);
            let signal = &service.inner.timers.get_timer(source).timer_signal;
            assert_eq!(signal.try_take(), Some(Some(5)));
            signal.signal(Some(5));
            poll_runner(runner.as_mut());
            assert_eq!(wake_level(service), Poll::Pending);
            assert!(!service.get_wake_status(source).timer_triggered_wake());

            advance_clock(service, 5);
            deliver_deadline(service, source, runner.as_mut());
            assert_eq!(wake_level(service), Poll::Ready(true));
        });
    }
}

#[test]
fn stale_expiry_does_not_skip_a_restarted_policy_delay() {
    with_service(|service, runner| {
        let source = AcpiTimerId::DcPower;
        service
            .set_expired_timer_policy(source, AlarmExpiredWakePolicy(10))
            .unwrap();
        service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
        assert_eq!(wake_level(service), Poll::Ready(false));
        let mut runner = pin!(runner.run());
        poll_runner(runner.as_mut());
        service.set_power_source(source);
        advance_clock(service, 3);
        service.set_power_source(AcpiTimerId::AcPower);
        advance_clock(service, 7);
        service.set_power_source(source);

        // The old deadline completed before a source change, but has not processed it yet.
        let timer = service.inner.timers.get_timer(source);
        assert!(!timer.process_expired_timer(&service.inner.clock_state));
        assert_eq!(timer.timer_signal.try_take(), Some(Some(7)));
        assert!(!service.get_wake_status(source).timer_triggered_wake());
        assert_eq!(wake_level(service), Poll::Pending);
        advance_clock(service, 7);
        deliver_deadline(service, source, runner.as_mut());
        assert_eq!(wake_level(service), Poll::Ready(true));
    });
}

#[test]
fn both_latches_are_ored_and_source_changes_do_not_acknowledge() {
    for first in SOURCES {
        with_service(|service, runner| {
            let mut runner = pin!(runner.run());
            latch_both(service, runner.as_mut());
            assert_eq!(wake_level(service), Poll::Ready(true));
            for source in SOURCES {
                service.set_power_source(source);
                assert!(service.get_wake_status(source).timer_triggered_wake());
            }
            assert_eq!(wake_level(service), Poll::Pending);
            service.clear_wake_status(first);
            assert_eq!(wake_level(service), Poll::Ready(true));
            service.clear_wake_status(first.get_other_timer_id());
            assert_eq!(wake_level(service), Poll::Ready(false));
        });
    }
}

#[test]
fn clear_rearm_reprogram_and_disable_update_the_level() {
    for source in SOURCES {
        with_service(|service, runner| {
            service.set_power_source(source);
            let mut runner = pin!(runner.run());
            for _ in 0..2 {
                service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
                assert_eq!(wake_level(service), Poll::Ready(false));
                poll_runner(runner.as_mut());
                assert_eq!(wake_level(service), Poll::Ready(true));
                service.clear_wake_status(source);
                assert_eq!(wake_level(service), Poll::Ready(false));
                assert_eq!(service.get_wake_status(source), Default::default());
            }
            for new_value in [AlarmTimerSeconds(30), AlarmTimerSeconds::DISABLED] {
                service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
                poll_runner(runner.as_mut());
                assert_eq!(wake_level(service), Poll::Ready(true));
                service.set_timer_value(source, new_value).unwrap();
                assert_eq!(wake_level(service), Poll::Ready(false));
                assert_eq!(service.get_wake_status(source), Default::default());
                assert_eq!(service.get_timer_value(source).unwrap(), new_value);
                poll_runner(runner.as_mut());
                assert_eq!(wake_level(service), Poll::Pending);
            }
        });
    }
}

#[test]
fn reprogram_and_disable_preserve_the_other_latch() {
    with_service(|service, runner| {
        let mut runner = pin!(runner.run());
        latch_both(service, runner.as_mut());
        service
            .set_timer_value(AcpiTimerId::AcPower, AlarmTimerSeconds(30))
            .unwrap();
        assert_eq!(wake_level(service), Poll::Ready(true));
        service
            .set_timer_value(AcpiTimerId::DcPower, AlarmTimerSeconds::DISABLED)
            .unwrap();
        assert_eq!(wake_level(service), Poll::Ready(false));
    });
}

#[test]
fn pending_publications_coalesce_to_the_latest_level() {
    with_service(|service, runner| {
        let source = AcpiTimerId::AcPower;
        let mut runner = pin!(runner.run());
        service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
        poll_runner(runner.as_mut());
        service.clear_wake_status(source);
        assert_eq!(wake_level(service), Poll::Ready(false));
        assert_eq!(wake_level(service), Poll::Pending);

        service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
        poll_runner(runner.as_mut());
        service.clear_wake_status(source);
        service.set_timer_value(source, AlarmTimerSeconds(0)).unwrap();
        poll_runner(runner.as_mut());
        assert_eq!(wake_level(service), Poll::Ready(true));
        assert!(service.get_wake_status(source).timer_triggered_wake());
    });
}

#[test]
fn concurrent_clears_cannot_leave_a_stale_asserted_level() {
    struct SharedService<'hw>(Service<'hw>);

    // SAFETY: This fixture's clock and NVRAM are Send, and all accesses use GlobalRawMutex.
    unsafe impl Sync for SharedService<'_> {}

    impl SharedService<'_> {
        fn clear(&self, source: AcpiTimerId) {
            self.0.clear_wake_status(source);
        }
    }

    with_service(|service, runner| {
        let shared = SharedService(service);
        let barrier = Barrier::new(2);
        let mut runner = pin!(runner.run());
        for _ in 0..128 {
            latch_both(service, runner.as_mut());
            std::thread::scope(|scope| {
                for source in SOURCES {
                    let shared = &shared;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        shared.clear(source);
                    });
                }
            });
            for source in SOURCES {
                assert_eq!(service.get_wake_status(source), Default::default());
            }
            assert_eq!(wake_level(service), Poll::Ready(false));
        }
    });
}
