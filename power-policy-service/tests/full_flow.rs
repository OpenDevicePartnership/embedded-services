//! Full flow test for the power policy service. This tests verifies multiple flows of the service with two separate devices.
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use embassy_sync::pubsub::DynSubscriber;
use embassy_time::{self as _, Timer};
use embedded_services::{
    info,
    power::policy::{self, ConsumerPowerCapability, ProviderPowerCapability, action, device, flags},
};

mod common;

use common::mock::Mock;

use crate::common::{
    DEFAULT_TIMEOUT, DEVICE0_ID, DEVICE1_ID, HIGH_POWER, LOW_POWER, PER_CALL_TIMEOUT, Test, assert_consumer_connected,
    assert_consumer_disconnected, assert_no_event, assert_provider_connected, assert_provider_disconnected,
    assert_unconstrained,
};

struct TestFullFlow;

impl Test for TestFullFlow {
    async fn run_test(
        &mut self,
        device0: action::device::Device<'static, action::Detached>,
        device0_mock: &'static Mock,
        device1: action::device::Device<'static, action::Detached>,
        device1_mock: &'static Mock,
        mut power_policy_event_receiver: DynSubscriber<'static, policy::CommsMessage>,
    ) {
        // Plug in device 0, should become current consumer
        info!("Connecting device 0");
        let device0 = device0.attach().await.unwrap();
        device0
            .notify_consumer_power_capability(Some(ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: flags::Consumer::none().with_unconstrained_power(),
            }))
            .await
            .unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert_eq!(
            device0_mock.messages.lock().await.pop_front().unwrap(),
            device::CommandData::ConnectAsConsumer(ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: flags::Consumer::none().with_unconstrained_power(),
            })
        );
        assert_consumer_connected(
            &mut power_policy_event_receiver,
            DEVICE0_ID,
            ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: flags::Consumer::none().with_unconstrained_power(),
            },
        )
        .await;
        assert_unconstrained(
            &mut power_policy_event_receiver,
            policy::UnconstrainedState {
                unconstrained: true,
                available: 1,
            },
        )
        .await;

        // Plug in device 1, should become current consumer
        info!("Connecting device 1");
        let device1 = device1.attach().await.unwrap();
        device1
            .notify_consumer_power_capability(Some(ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: flags::Consumer::none(),
            }))
            .await
            .unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert_eq!(
            device0_mock.messages.lock().await.pop_front().unwrap(),
            device::CommandData::Disconnect
        );
        assert_eq!(
            device1_mock.messages.lock().await.pop_front().unwrap(),
            device::CommandData::ConnectAsConsumer(ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: flags::Consumer::none(),
            })
        );
        assert_consumer_disconnected(&mut power_policy_event_receiver, DEVICE0_ID).await;
        assert_consumer_connected(
            &mut power_policy_event_receiver,
            DEVICE1_ID,
            ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: flags::Consumer::none(),
            },
        )
        .await;

        // Unplug device 0, device 1 should remain current consumer
        info!("Unplugging device 0");
        let device0 = device0.detach().await.unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.pop_front().is_none());
        assert!(device1_mock.messages.lock().await.pop_front().is_none());
        assert_unconstrained(
            &mut power_policy_event_receiver,
            policy::UnconstrainedState {
                unconstrained: false,
                available: 1,
            },
        )
        .await;

        // Plug in device 0, device 1 should remain current consumer
        info!("Connecting device 0");
        let device0 = device0.attach().await.unwrap();
        device0
            .notify_consumer_power_capability(Some(LOW_POWER.into()))
            .await
            .unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.pop_front().is_none());
        assert!(device1_mock.messages.lock().await.pop_front().is_none());
        assert_unconstrained(
            &mut power_policy_event_receiver,
            policy::UnconstrainedState {
                unconstrained: false,
                available: 0,
            },
        )
        .await;

        // Unplug device 1, device 0 should become current consumer
        info!("Unplugging device 1");
        let device1 = device1.detach().await.unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert_eq!(
            device0_mock.messages.lock().await.pop_front(),
            Some(device::CommandData::ConnectAsConsumer(ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: flags::Consumer::none(),
            }))
        );
        assert!(device1_mock.messages.lock().await.pop_front().is_none());
        assert_consumer_disconnected(&mut power_policy_event_receiver, DEVICE1_ID).await;
        assert_consumer_connected(
            &mut power_policy_event_receiver,
            DEVICE0_ID,
            ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: flags::Consumer::none(),
            },
        )
        .await;

        // Replug device 1, device 1 becomes current consumer
        info!("Connecting device 1");
        let device1 = device1.attach().await.unwrap();
        device1
            .notify_consumer_power_capability(Some(HIGH_POWER.into()))
            .await
            .unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert_eq!(
            device0_mock.messages.lock().await.pop_front(),
            Some(device::CommandData::Disconnect)
        );
        assert_eq!(
            device1_mock.messages.lock().await.pop_front(),
            Some(device::CommandData::ConnectAsConsumer(ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: flags::Consumer::none(),
            }))
        );
        assert_consumer_disconnected(&mut power_policy_event_receiver, DEVICE0_ID).await;
        assert_consumer_connected(
            &mut power_policy_event_receiver,
            DEVICE1_ID,
            ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: flags::Consumer::none(),
            },
        )
        .await;

        // Signal no consumer capability for device 0, device 1 should remain current consumer
        // Device 0 should not consume after device 1 is unplugged
        info!("Disconnecting device 0");
        device0.notify_consumer_power_capability(None).await.unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.is_empty());
        assert!(device1_mock.messages.lock().await.is_empty());
        assert_no_event(&mut power_policy_event_receiver);

        let device1 = device1.detach().await.unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.is_empty());
        assert!(device1_mock.messages.lock().await.is_empty());
        assert_consumer_disconnected(&mut power_policy_event_receiver, DEVICE1_ID).await;

        // Switch to provider on device0
        info!("Device 0 requesting provider");
        device0
            .request_provider_power_capability(LOW_POWER.into())
            .await
            .unwrap();
        Timer::after(PER_CALL_TIMEOUT).await;

        assert_eq!(
            device0_mock.messages.lock().await.pop_front(),
            Some(device::CommandData::ConnectAsProvider(ProviderPowerCapability {
                capability: LOW_POWER,
                flags: flags::Provider::none(),
            }))
        );
        assert!(device1_mock.messages.lock().await.is_empty());
        assert_provider_connected(
            &mut power_policy_event_receiver,
            DEVICE0_ID,
            ProviderPowerCapability {
                capability: LOW_POWER,
                flags: flags::Provider::none(),
            },
        )
        .await;
        assert_eq!(policy::policy::compute_total_provider_power_mw().await, 7500,);

        info!("Device 1 attach and requesting provider");
        let device1 = device1.attach().await.unwrap();
        device1
            .request_provider_power_capability(LOW_POWER.into())
            .await
            .unwrap();
        // Wait for the provider to be connected
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.is_empty());
        assert_eq!(
            device1_mock.messages.lock().await.pop_front(),
            Some(device::CommandData::ConnectAsProvider(ProviderPowerCapability {
                capability: LOW_POWER,
                flags: flags::Provider::none(),
            }))
        );
        assert_provider_connected(
            &mut power_policy_event_receiver,
            DEVICE1_ID,
            ProviderPowerCapability {
                capability: LOW_POWER,
                flags: flags::Provider::none(),
            },
        )
        .await;
        assert_eq!(policy::policy::compute_total_provider_power_mw().await, 15000,);

        // Provider upgrade should fail because device 0 is already connected
        info!("Device 1 attempting provider upgrade");
        device1
            .request_provider_power_capability(HIGH_POWER.into())
            .await
            .unwrap();
        // Wait for the upgrade flow to complete
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.is_empty());
        assert_eq!(
            device1_mock.messages.lock().await.pop_front(),
            Some(device::CommandData::ConnectAsProvider(ProviderPowerCapability {
                capability: LOW_POWER,
                flags: flags::Provider::none()
            }))
        );
        assert_provider_connected(
            &mut power_policy_event_receiver,
            DEVICE1_ID,
            ProviderPowerCapability {
                capability: LOW_POWER,
                flags: flags::Provider::none(),
            },
        )
        .await;

        // Disconnect device 0
        info!("Device 0 disconnecting");
        device0.detach().await.unwrap();
        // Wait for the detach flow to complete
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.is_empty());
        assert!(device1_mock.messages.lock().await.is_empty());
        assert_provider_disconnected(&mut power_policy_event_receiver, DEVICE0_ID).await;
        assert_eq!(policy::policy::compute_total_provider_power_mw().await, 7500);

        // Provider upgrade should succeed now
        info!("Device 1 attempting provider upgrade");
        device1
            .request_provider_power_capability(HIGH_POWER.into())
            .await
            .unwrap();
        // Wait for the upgrade flow to complete
        Timer::after(PER_CALL_TIMEOUT).await;

        assert!(device0_mock.messages.lock().await.is_empty());
        assert_eq!(
            device1_mock.messages.lock().await.pop_front(),
            Some(device::CommandData::ConnectAsProvider(ProviderPowerCapability {
                capability: HIGH_POWER,
                flags: flags::Provider::none(),
            }))
        );
        assert_provider_connected(
            &mut power_policy_event_receiver,
            DEVICE1_ID,
            ProviderPowerCapability {
                capability: HIGH_POWER,
                flags: flags::Provider::none(),
            },
        )
        .await;
        assert_eq!(policy::policy::compute_total_provider_power_mw().await, 15000);

        assert_no_event(&mut power_policy_event_receiver);
        assert!(device0_mock.messages.lock().await.is_empty());
        assert!(device1_mock.messages.lock().await.is_empty());
    }
}

#[tokio::test]
async fn full_flow() {
    common::run_test(
        TestFullFlow,
        DEFAULT_TIMEOUT,
        power_policy_service::config::Config::default(),
    )
    .await;
}
