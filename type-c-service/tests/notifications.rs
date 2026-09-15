#![allow(dead_code)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use embassy_futures::join::join;
use embassy_time::{TimeoutError, with_timeout};
use embedded_usb_pd::{ado::Ado, type_c::ConnectionState};
use type_c_interface::{
    control::dp::{DpPinConfig, DpStatus},
    control::pd::PortStatus,
    control::vdm::{ATTN_VDM_LEN, AttnVdm, OTHER_VDM_LEN, OtherVdm},
    port::event::{PortEvent, PortStatusEventBitfield, VdmData, VdmNotification},
    service::event::PortEventData,
};
use type_c_interface_test_mocks::controller::{FnCall as ControllerFnCall, pd::FnCall as PdFnCall};
use type_c_service::controller::event::Event;

use crate::common::{
    DEFAULT_PER_CALL_TIMEOUT, DEFAULT_TEST_DURATION, PowerPolicyServiceReceiver, Test, TestPort, TypeCServiceReceiver,
};

mod common;

/// Assert that neither the type-C service nor the power policy service broadcast an event.
///
/// PD alerts, VDMs, and DP status updates are purely informational at the port level, so they
/// must never leak out as a type-C service or power policy service broadcast.
async fn assert_no_service_broadcast(
    type_c_receiver: &TypeCServiceReceiver<'_, '_>,
    power_policy_receiver: &PowerPolicyServiceReceiver<'_, '_>,
) {
    let (type_c_result, power_policy_result) = join(
        with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
        with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
    )
    .await;
    assert_eq!(
        type_c_result.err(),
        Some(TimeoutError),
        "unexpected type-C service broadcast"
    );
    assert_eq!(
        power_policy_result.err(),
        Some(TimeoutError),
        "unexpected power policy broadcast"
    );
}

/// Test the PD alert flow.
///
/// When the controller reports an alert, the port should retrieve the ADO and surface it as a
/// [`PortEventData::Alert`] event. When the controller reports no alert, the port should not
/// produce an event.
struct TestPdAlert;

impl Test for TestPdAlert {
    async fn run<'port, 'ch>(
        &mut self,
        _service: &common::TypeCServiceMutexType<'port, 'ch>,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // The controller reports a power button press alert.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0
                .next_result_get_pd_alert
                .push_back(Ok(Some(Ado::PowerButtonPress)));
        }

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::Alert))
            .await
            .unwrap();

        // The port should surface the alert as an `Alert` event carrying the reported ADO.
        match result {
            Some(PortEventData::Alert(ado)) => assert_eq!(ado, Ado::PowerButtonPress),
            other => panic!("Expected PortEventData::Alert, got {other:?}"),
        }

        // The controller's `get_pd_alert` should have been called exactly once.
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetPdAlert(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }

        // A PD alert is informational and must not trigger any service broadcasts.
        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;

        // The controller reports no alert; the port should not produce an event.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_pd_alert.push_back(Ok(None));
        }

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::Alert))
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "Expected no event when the controller reports no alert"
        );

        // No-alert is also informational and must not trigger any service broadcasts.
        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;

        // The controller should still have been queried for the alert.
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetPdAlert(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }
    }
}

/// Test the VDM flow.
///
/// Each [`VdmNotification`] should be translated into the matching [`VdmData`] variant by
/// retrieving the relevant VDM payload from the controller. "Other" VDMs (entered, exited,
/// received) are read via `get_other_vdm`; attention VDMs are read via `get_attn_vdm`.
struct TestVdm;

impl Test for TestVdm {
    async fn run<'port, 'ch>(
        &mut self,
        _service: &common::TypeCServiceMutexType<'port, 'ch>,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // --- Custom mode entered ---
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_other_vdm.push_back(Ok(OtherVdm {
                data: [0x11; OTHER_VDM_LEN],
            }));
        }

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::Vdm(VdmNotification::Entered)))
            .await
            .unwrap();
        match result {
            Some(PortEventData::Vdm(VdmData::Entered(vdm))) => assert_eq!(vdm.data, [0x11; OTHER_VDM_LEN]),
            other => panic!("Expected VdmData::Entered, got {other:?}"),
        }
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetOtherVdm(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }

        // --- Custom mode exited ---
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_other_vdm.push_back(Ok(OtherVdm {
                data: [0x22; OTHER_VDM_LEN],
            }));
        }

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::Vdm(VdmNotification::Exited)))
            .await
            .unwrap();
        match result {
            Some(PortEventData::Vdm(VdmData::Exited(vdm))) => assert_eq!(vdm.data, [0x22; OTHER_VDM_LEN]),
            other => panic!("Expected VdmData::Exited, got {other:?}"),
        }
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetOtherVdm(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }

        // --- Other VDM received ---
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_other_vdm.push_back(Ok(OtherVdm {
                data: [0x33; OTHER_VDM_LEN],
            }));
        }

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::Vdm(VdmNotification::OtherReceived)))
            .await
            .unwrap();
        match result {
            Some(PortEventData::Vdm(VdmData::ReceivedOther(vdm))) => assert_eq!(vdm.data, [0x33; OTHER_VDM_LEN]),
            other => panic!("Expected VdmData::ReceivedOther, got {other:?}"),
        }
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetOtherVdm(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }

        // --- Attention VDM received ---
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_attn_vdm.push_back(Ok(AttnVdm {
                data: [0x44; ATTN_VDM_LEN],
            }));
        }

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::Vdm(VdmNotification::AttentionReceived)))
            .await
            .unwrap();
        match result {
            Some(PortEventData::Vdm(VdmData::ReceivedAttn(vdm))) => assert_eq!(vdm.data, [0x44; ATTN_VDM_LEN]),
            other => panic!("Expected VdmData::ReceivedAttn, got {other:?}"),
        }
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetAttnVdm(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }

        // VDMs are informational and must not trigger any service broadcasts.
        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;
    }
}

/// Test the DisplayPort status update flow.
///
/// A DP status update should retrieve the current [`DpStatus`] from the controller and surface it
/// as a [`PortEventData::DpStatusUpdate`] event.
struct TestDpStatus;

impl Test for TestDpStatus {
    async fn run<'port, 'ch>(
        &mut self,
        _service: &common::TypeCServiceMutexType<'port, 'ch>,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        let expected_status = DpStatus {
            alt_mode_entered: true,
            dfp_d_pin_cfg: DpPinConfig {
                pin_c: true,
                pin_d: false,
                pin_e: false,
            },
        };

        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_dp_status.push_back(Ok(expected_status));
        }

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::DpStatusUpdate))
            .await
            .unwrap();
        match result {
            Some(PortEventData::DpStatusUpdate(status)) => assert_eq!(status, expected_status),
            other => panic!("Expected PortEventData::DpStatusUpdate, got {other:?}"),
        }

        // The controller's `get_dp_status` should have been called exactly once.
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetDpStatus(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }

        // A DP status update is informational and must not trigger any service broadcasts.
        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;
    }
}

/// Test the PD hard reset flow.
///
/// A hard reset arrives as a status-changed event with the `pd_hard_reset` bit set. The port
/// should re-read the port status from the controller and preserve the hard reset flag in the
/// emitted [`PortEventData::StatusChanged`] event so that downstream consumers (e.g. UCSI) can
/// report the reset.
struct TestHardReset;

impl Test for TestHardReset {
    async fn run<'port, 'ch>(
        &mut self,
        _service: &common::TypeCServiceMutexType<'port, 'ch>,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // A hard reset occurs while the port is connected.
        let port_status = PortStatus {
            connection_state: Some(ConnectionState::Attached),
            ..Default::default()
        };
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(port_status));
        }

        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_pd_hard_reset(true);

        let result = port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(status_event)))
            .await
            .unwrap();

        // The port should re-read the port status from the controller.
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(matches!(
                mock0.fn_calls.pop_front(),
                Some(ControllerFnCall::Pd(PdFnCall::GetPortStatus(_)))
            ));
            assert!(mock0.fn_calls.is_empty());
        }

        // The hard reset flag should be preserved in the emitted status-changed event.
        match result {
            Some(PortEventData::StatusChanged(data)) => {
                assert!(data.status_event.pd_hard_reset(), "hard reset flag should be set");
                assert_eq!(data.current_status, port_status);
            }
            other => panic!("Expected PortEventData::StatusChanged, got {other:?}"),
        }

        // With UCSI notifications disabled (the default), the hard reset should not surface as a
        // type-C service or power policy service broadcast.
        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;
    }
}

#[tokio::test]
async fn test_pd_alert() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestPdAlert,
    )
    .await;
}

#[tokio::test]
async fn test_vdm() {
    common::run_test(DEFAULT_TEST_DURATION, Default::default(), Default::default(), TestVdm).await;
}

#[tokio::test]
async fn test_dp_status() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestDpStatus,
    )
    .await;
}

#[tokio::test]
async fn test_hard_reset() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestHardReset,
    )
    .await;
}
