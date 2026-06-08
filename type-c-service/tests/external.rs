//! Integration test for external function calls

mod common;

use core::num::NonZeroU8;

use common::Test;
use embassy_futures::join::join3;
use embassy_sync::{mutex::Mutex, pubsub::DynSubscriber};
use embassy_time::with_timeout;
use embedded_services::{
    GlobalRawMutex, info,
    power::policy,
    type_c::{
        self, Cached, ControllerId,
        controller::{
            ControllerStatus, DiscoveredSvids, DpConfig, DpPinConfig, DpStatus, PdStateMachineConfig, PortStatus,
            RetimerFwUpdateState, SendVdm, SystemPowerState, TbtConfig, TypeCStateMachineState, UsbControlConfig,
        },
        external,
    },
};
use embedded_usb_pd::{
    GlobalPortId, LocalPortId,
    usb::{Bcd, ProductId},
    vdm::structured::command::discover_identity::{
        CertStatVdo, ConnectorType, IdHeaderVdo, ProductVdo,
        sop::{self, DfpProductTypeVdos, UfpProductTypeVdos},
        sop_prime::{self, ProductTypeVdos},
    },
};

use common::DEFAULT_TEST_DURATION;
use common::mock;

use crate::common::DEFAULT_PER_CALL_TIMEOUT;

struct TestExternal;

impl TestExternal {
    async fn run_tests(
        &self,
        controller_id: ControllerId,
        port_id: GlobalPortId,
        port: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
    ) {
        let local_port_id = LocalPortId(0);

        // get_controller_status
        info!("Testing get_controller_status");
        let expected_controller_status = ControllerStatus {
            mode: "Test",
            valid_fw_bank: true,
            fw_version0: 0xbadbeef,
            fw_version1: 0xbadcafe,
        };
        port.lock()
            .await
            .next_result_get_controller_status
            .push_back(Ok(expected_controller_status));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::get_controller_status(controller_id)).await,
            Ok(Ok(expected_controller_status))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetControllerStatus)
        );

        // reset_controller
        info!("Testing reset_controller");
        port.lock().await.next_result_reset_controller.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::reset_controller(controller_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::ResetController)
        );

        // sync_controller_state
        // The service fetches port status as a side-effect of syncing state.
        info!("Testing sync_controller_state");
        port.lock()
            .await
            .next_result_get_port_status
            .push_back(Ok(PortStatus::default()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::sync_controller_state(controller_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetPortStatus(local_port_id))
        );

        // get_controller_num_ports
        // Each test controller is registered with one port.
        info!("Testing get_controller_num_ports");
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::get_controller_num_ports(controller_id)
            )
            .await,
            Ok(Ok(1))
        );

        // controller_port_to_global_id
        info!("Testing controller_port_to_global_id");
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::controller_port_to_global_id(controller_id, local_port_id)
            )
            .await,
            Ok(Ok(port_id)),
        );

        // global_port_to_controller_port
        info!("Testing global_port_to_controller_port");
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::global_port_to_controller_port(port_id)
            )
            .await,
            Ok(Ok((controller_id, local_port_id))),
        );

        // get_num_ports
        // Three controllers are registered in the test harness.
        info!("Testing get_num_ports");
        assert_eq!(external::get_num_ports(), 3);

        // get_port_status
        info!("Testing get_port_status");
        let expected_port_status = PortStatus::default();
        port.lock()
            .await
            .next_result_get_port_status
            .push_back(Ok(expected_port_status));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::get_port_status(port_id, Cached(false))
            )
            .await,
            Ok(Ok(expected_port_status)),
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetPortStatus(local_port_id))
        );

        // get_controller_port_status
        info!("Testing get_controller_port_status");
        port.lock()
            .await
            .next_result_get_port_status
            .push_back(Ok(expected_port_status));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::get_controller_port_status(controller_id, local_port_id, Cached(false))
            )
            .await,
            Ok(Ok(expected_port_status)),
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetPortStatus(local_port_id))
        );

        // port_get_rt_fw_update_status
        info!("Testing port_get_rt_fw_update_status");
        port.lock()
            .await
            .next_result_get_rt_fw_update_status
            .push_back(Ok(RetimerFwUpdateState::Active));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::port_get_rt_fw_update_status(port_id)
            )
            .await,
            Ok(Ok(RetimerFwUpdateState::Active)),
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetRtFwUpdateStatus(local_port_id))
        );

        // port_set_rt_fw_update_state
        info!("Testing port_set_rt_fw_update_state");
        port.lock().await.next_result_set_rt_fw_update_state.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::port_set_rt_fw_update_state(port_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetRtFwUpdateState(local_port_id))
        );

        // port_clear_rt_fw_update_state
        info!("Testing port_clear_rt_fw_update_state");
        port.lock().await.next_result_clear_rt_fw_update_state.push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::port_clear_rt_fw_update_state(port_id)
            )
            .await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::ClearRtFwUpdateState(local_port_id))
        );

        // port_set_rt_compliance
        info!("Testing port_set_rt_compliance");
        port.lock().await.next_result_set_rt_compliance.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::port_set_rt_compliance(port_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetRtCompliance(local_port_id))
        );

        // reconfigure_retimer
        info!("Testing reconfigure_retimer");
        port.lock().await.next_result_reconfigure_retimer.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::reconfigure_retimer(port_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::ReconfigureRetimer(local_port_id))
        );

        // set_max_sink_voltage
        info!("Testing set_max_sink_voltage");
        port.lock().await.next_result_set_max_sink_voltage.push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::set_max_sink_voltage(port_id, Some(5000))
            )
            .await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetMaxSinkVoltage(local_port_id, Some(5000)))
        );

        // clear_dead_battery_flag
        info!("Testing clear_dead_battery_flag");
        port.lock().await.next_result_clear_dead_battery_flag.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::clear_dead_battery_flag(port_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::ClearDeadBatteryFlag(local_port_id))
        );

        // set_power_state
        info!("Testing set_power_state");
        port.lock().await.next_result_set_power_state.push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::set_power_state(port_id, SystemPowerState::S0)
            )
            .await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetPowerState(local_port_id, SystemPowerState::S0))
        );

        // execute_electrical_disconnect
        info!("Testing execute_electrical_disconnect");
        port.lock()
            .await
            .next_result_execute_electrical_disconnect
            .push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::execute_electrical_disconnect(port_id, NonZeroU8::new(5))
            )
            .await,
            Ok(Ok(())),
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::ExecuteElectricalDisconnect(
                local_port_id,
                NonZeroU8::new(5)
            ))
        );

        // send_vdm
        info!("Testing send_vdm");
        port.lock().await.next_result_send_vdm.push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::send_vdm(port_id, SendVdm::default())
            )
            .await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SendVdm(local_port_id, SendVdm::default()))
        );

        // set_usb_control
        info!("Testing set_usb_control");
        port.lock().await.next_result_set_usb_control.push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::set_usb_control(port_id, UsbControlConfig::default())
            )
            .await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetUsbControl(local_port_id, UsbControlConfig::default()))
        );

        // get_dp_status
        info!("Testing get_dp_status");
        let expected_dp_status = DpStatus {
            alt_mode_entered: true,
            dfp_d_pin_cfg: DpPinConfig {
                pin_c: true,
                pin_d: false,
                pin_e: false,
            },
        };
        port.lock()
            .await
            .next_result_get_dp_status
            .push_back(Ok(expected_dp_status));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::get_dp_status(port_id)).await,
            Ok(Ok(expected_dp_status))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetDpStatus(local_port_id))
        );

        // set_dp_config
        info!("Testing set_dp_config");
        let dp_config = DpConfig {
            enable: true,
            dfp_d_pin_cfg: DpPinConfig::default(),
        };
        port.lock().await.next_result_set_dp_config.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::set_dp_config(port_id, dp_config)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetDpConfig(local_port_id, dp_config))
        );

        // execute_drst
        info!("Testing execute_drst");
        port.lock().await.next_result_execute_drst.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::execute_drst(port_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::ExecuteDrst(local_port_id))
        );

        // set_tbt_config
        info!("Testing set_tbt_config");
        let tbt_config = TbtConfig { tbt_enabled: true };
        port.lock().await.next_result_set_tbt_config.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::set_tbt_config(port_id, tbt_config)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetTbtConfig(local_port_id, tbt_config))
        );

        // set_pd_state_machine_config
        info!("Testing set_pd_state_machine_config");
        let pd_sm_config = PdStateMachineConfig { enabled: true };
        port.lock()
            .await
            .next_result_set_pd_state_machine_config
            .push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::set_pd_state_machine_config(port_id, pd_sm_config)
            )
            .await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetPdStateMachineConfig(local_port_id, pd_sm_config))
        );

        // set_type_c_state_machine_config
        info!("Testing set_type_c_state_machine_config");
        port.lock()
            .await
            .next_result_set_type_c_state_machine_config
            .push_back(Ok(()));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::set_type_c_state_machine_config(port_id, TypeCStateMachineState::Drp)
            )
            .await,
            Ok(Ok(())),
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::SetTypeCStateMachineConfig(
                local_port_id,
                TypeCStateMachineState::Drp
            ))
        );

        // get_discovered_svids
        info!("Testing get_discovered_svids");
        let expected_svids = DiscoveredSvids::default();
        port.lock()
            .await
            .next_result_get_discovered_svids
            .push_back(Ok(expected_svids));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::get_discovered_svids(port_id)).await,
            Ok(Ok(expected_svids))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetDiscoveredSvids(local_port_id))
        );

        // hard_reset
        info!("Testing hard_reset");
        port.lock().await.next_result_hard_reset.push_back(Ok(()));
        assert_eq!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, external::hard_reset(port_id)).await,
            Ok(Ok(()))
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::HardReset(local_port_id))
        );

        // get_discover_identity_sop_response
        info!("Testing get_discover_identity_sop_response");
        let expected_sop_identity = sop::ResponseVdos {
            id: IdHeaderVdo {
                usb_vendor_id: 0x1234,
                connector_type: ConnectorType::Plug,
                modal_operation_supported: false,
                usb_communication_capable_as_usb_device: true,
                usb_communication_capable_as_usb_host: false,
            },
            cert_stat: CertStatVdo(0x12345678),
            product: ProductVdo {
                usb_product_id: ProductId(0x5678),
                bcd_device: Bcd(0x0100),
            },
            dfp_product_type_vdos: DfpProductTypeVdos::NotADfp,
            ufp_product_type_vdos: UfpProductTypeVdos::Psd,
        };
        port.lock()
            .await
            .next_result_get_discover_identity_sop_response
            .push_back(Ok(expected_sop_identity));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::get_discover_identity_sop_response(port_id)
            )
            .await,
            Ok(Ok(expected_sop_identity)),
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetDiscoverIdentitySopResponse(local_port_id))
        );

        // get_discover_identity_sop_prime_response
        info!("Testing get_discover_identity_sop_prime_response");
        let expected_sop_prime_identity = sop_prime::ResponseVdos {
            id: IdHeaderVdo {
                usb_vendor_id: 0x1234,
                connector_type: ConnectorType::Plug,
                modal_operation_supported: false,
                usb_communication_capable_as_usb_device: true,
                usb_communication_capable_as_usb_host: false,
            },
            cert_stat: CertStatVdo(0x12345678),
            product: ProductVdo {
                usb_product_id: ProductId(0x5678),
                bcd_device: Bcd(0x0100),
            },
            product_type_vdos: ProductTypeVdos::NotACablePlugVpd,
        };
        port.lock()
            .await
            .next_result_get_discover_identity_sop_prime_response
            .push_back(Ok(expected_sop_prime_identity));
        assert_eq!(
            with_timeout(
                DEFAULT_PER_CALL_TIMEOUT,
                external::get_discover_identity_sop_prime_response(port_id)
            )
            .await,
            Ok(Ok(expected_sop_prime_identity)),
        );
        assert_eq!(
            port.lock().await.fn_calls.pop_front(),
            Some(mock::FnCall::GetDiscoverIdentitySopPrimeResponse(local_port_id))
        );
    }
}

impl Test for TestExternal {
    async fn run(
        &mut self,
        _type_c_receiver: DynSubscriber<'static, type_c::comms::CommsMessage>,
        _power_policy_event_receiver: DynSubscriber<'static, policy::CommsMessage>,
        port0: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        port1: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        port2: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
    ) {
        join3(
            self.run_tests(ControllerId(0), GlobalPortId(0), port0),
            self.run_tests(ControllerId(1), GlobalPortId(1), port1),
            self.run_tests(ControllerId(2), GlobalPortId(2), port2),
        )
        .await;
    }
}

#[tokio::test]
async fn external() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        type_c_service::service::config::Config::default(),
        power_policy_service::config::Config::default(),
        TestExternal,
    )
    .await;
}
