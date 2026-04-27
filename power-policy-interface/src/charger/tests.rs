use super::*;
use crate::capability::{ConsumerFlags, PowerCapability};

fn cap(voltage_mv: u16, current_ma: u16) -> ConsumerPowerCapability {
    ConsumerPowerCapability {
        capability: PowerCapability { voltage_mv, current_ma },
        flags: ConsumerFlags::none(),
    }
}

fn state_init() -> State {
    State {
        state: InternalState::Powered(PoweredSubstate::Init),
        capability: None,
    }
}

fn state_psu_attached() -> State {
    State {
        state: InternalState::Powered(PoweredSubstate::PsuAttached),
        capability: None,
    }
}

fn state_psu_detached() -> State {
    State {
        state: InternalState::Powered(PoweredSubstate::PsuDetached),
        capability: None,
    }
}

fn state_unpowered() -> State {
    State::default()
}

// on_initialized

#[test]
fn on_initialized_from_init_attached() {
    let mut s = state_init();
    assert!(s.on_initialized(PsuState::Attached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));
}

#[test]
fn on_initialized_from_init_detached() {
    let mut s = state_init();
    assert!(s.on_initialized(PsuState::Detached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuDetached));
}

#[test]
fn on_initialized_from_psu_attached_fails() {
    let mut s = state_psu_attached();
    assert_eq!(
        s.on_initialized(PsuState::Attached),
        Err(ChargerError::InvalidState(InternalState::Powered(
            PoweredSubstate::PsuAttached
        )))
    );
}

#[test]
fn on_initialized_from_unpowered_fails() {
    let mut s = state_unpowered();
    assert_eq!(
        s.on_initialized(PsuState::Attached),
        Err(ChargerError::InvalidState(InternalState::Unpowered))
    );
}

// on_psu_state_change

#[test]
fn psu_state_change_attached_to_detached() {
    let mut s = state_psu_attached();
    assert!(s.on_psu_state_change(PsuState::Detached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuDetached));
}

#[test]
fn psu_state_change_detached_to_attached() {
    let mut s = state_psu_detached();
    assert!(s.on_psu_state_change(PsuState::Attached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));
}

#[test]
fn psu_state_change_same_state_is_noop() {
    let mut s = state_psu_attached();
    assert!(s.on_psu_state_change(PsuState::Attached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));
}

#[test]
fn psu_state_change_from_init_fails() {
    let mut s = state_init();
    assert_eq!(
        s.on_psu_state_change(PsuState::Attached),
        Err(ChargerError::InvalidState(InternalState::Powered(
            PoweredSubstate::Init
        )))
    );
}

#[test]
fn psu_state_change_from_unpowered_fails() {
    let mut s = state_unpowered();
    assert_eq!(
        s.on_psu_state_change(PsuState::Attached),
        Err(ChargerError::InvalidState(InternalState::Unpowered))
    );
}

// on_timeout

#[test]
fn timeout_from_psu_attached() {
    let mut s = state_psu_attached();
    s.capability = Some(cap(5000, 3000));
    s.on_timeout();
    assert_eq!(s.state, InternalState::Unpowered);
    assert!(s.capability.is_none());
}

#[test]
fn timeout_from_psu_detached() {
    let mut s = state_psu_detached();
    s.on_timeout();
    assert_eq!(s.state, InternalState::Unpowered);
}

#[test]
fn timeout_from_init() {
    let mut s = state_init();
    s.on_timeout();
    assert_eq!(s.state, InternalState::Unpowered);
}

#[test]
fn timeout_from_unpowered() {
    let mut s = state_unpowered();
    s.on_timeout();
    assert_eq!(s.state, InternalState::Unpowered);
}

// on_ready_success

#[test]
fn ready_success_from_unpowered() {
    let mut s = state_unpowered();
    s.on_ready_success();
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::Init));
    assert!(s.capability.is_none());
}

#[test]
fn ready_success_from_powered_is_noop() {
    let mut s = state_psu_attached();
    s.capability = Some(cap(5000, 3000));
    s.on_ready_success();
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));
    assert!(s.capability.is_some());
}

// on_ready_failure

#[test]
fn ready_failure_from_powered() {
    let mut s = state_psu_attached();
    s.capability = Some(cap(5000, 3000));
    s.on_ready_failure();
    assert_eq!(s.state, InternalState::Unpowered);
    assert!(s.capability.is_some()); // preserved for diagnostics
}

#[test]
fn ready_failure_from_unpowered_is_noop() {
    let mut s = state_unpowered();
    s.on_ready_failure();
    assert_eq!(s.state, InternalState::Unpowered);
}

// on_policy_attach

#[test]
fn policy_attach_from_psu_attached() {
    let mut s = state_psu_attached();
    let c = cap(5000, 3000);
    s.on_policy_attach(c);
    assert_eq!(s.capability, Some(c));
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));
}

#[test]
fn policy_attach_from_psu_detached() {
    let mut s = state_psu_detached();
    let c = cap(9000, 2000);
    s.on_policy_attach(c);
    assert_eq!(s.capability, Some(c));
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuDetached));
}

#[test]
fn policy_attach_from_init() {
    let mut s = state_init();
    let c = cap(5000, 3000);
    s.on_policy_attach(c);
    assert_eq!(s.capability, Some(c));
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::Init));
}

#[test]
fn policy_attach_from_unpowered() {
    let mut s = state_unpowered();
    let c = cap(5000, 3000);
    s.on_policy_attach(c);
    assert_eq!(s.capability, Some(c));
    assert_eq!(s.state, InternalState::Unpowered);
}

// on_policy_detach

#[test]
fn policy_detach_from_psu_attached() {
    let mut s = state_psu_attached();
    s.capability = Some(cap(5000, 3000));
    s.on_policy_detach();
    assert!(s.capability.is_none());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));
}

#[test]
fn policy_detach_from_init() {
    let mut s = state_init();
    s.capability = Some(cap(5000, 3000));
    s.on_policy_detach();
    assert!(s.capability.is_none());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::Init));
}

#[test]
fn policy_detach_from_unpowered() {
    let mut s = state_unpowered();
    s.capability = Some(cap(5000, 3000));
    s.on_policy_detach();
    assert!(s.capability.is_none());
    assert_eq!(s.state, InternalState::Unpowered);
}

// Full transition sequence

#[test]
fn full_lifecycle_unpowered_to_charging_and_back() {
    let mut s = state_unpowered();

    // Check ready → powered init
    s.on_ready_success();
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::Init));

    // Initialized with PSU attached
    assert!(s.on_initialized(PsuState::Attached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));

    // Policy attach
    let c = cap(5000, 3000);
    s.on_policy_attach(c);
    assert_eq!(s.capability, Some(c));

    // PSU detach
    assert!(s.on_psu_state_change(PsuState::Detached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuDetached));

    // Policy detach
    s.on_policy_detach();
    assert!(s.capability.is_none());

    // PSU reattach
    assert!(s.on_psu_state_change(PsuState::Attached).is_ok());
    assert_eq!(s.state, InternalState::Powered(PoweredSubstate::PsuAttached));

    // Timeout → unpowered
    s.on_timeout();
    assert_eq!(s.state, InternalState::Unpowered);
}
