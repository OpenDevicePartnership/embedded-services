//! Code related to simplifying the creation of backing channels and buffers for the wrapper.

use core::{
    array::from_fn,
    future::Future,
    ops::{Deref, DerefMut},
};

use embassy_sync::{
    blocking_mutex::raw::RawMutex,
    pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel},
};
use embedded_usb_pd::ado::Ado;

/// Trait for any struct that can provide backing storage for the controller wrapper.
pub trait Backing<'a> {
    /// Get the PD alert channel for a specific port index.
    fn pd_alert_channel(
        &self,
        port_index: usize,
    ) -> impl Future<Output = Option<impl Deref<Target = (DynImmediatePublisher<'a, Ado>, DynSubscriber<'a, Ado>)>>>;
    /// Get the PD alert channel for a specific port index.
    fn pd_alert_channel_mut(
        &mut self,
        port_index: usize,
    ) -> impl Future<Output = Option<impl DerefMut<Target = (DynImmediatePublisher<'a, Ado>, DynSubscriber<'a, Ado>)>>>;
}

/// PD alerts should be fairly uncommon, four seems like a reasonable number to start with.
const MAX_BUFFERED_PD_ALERTS: usize = 4;

/// Actual backing channels and buffers
pub struct BackingDefaultStorage<const N: usize, M: RawMutex> {
    /// PD alert channels for each port
    // 0 PUBS because we only use immediate publishers for PD alerts
    pd_alerts: [PubSubChannel<M, Ado, MAX_BUFFERED_PD_ALERTS, 1, 0>; N],
}

impl<const N: usize, M: RawMutex> BackingDefaultStorage<N, M> {
    pub const fn new() -> Self {
        Self {
            pd_alerts: [const { PubSubChannel::new() }; N],
        }
    }

    pub fn get_backing(&self) -> Option<BackingDefault<'_, N>> {
        let pd_alerts: [_; N] = from_fn(|i| {
            let publisher = self.pd_alerts[i].dyn_immediate_publisher();
            let subscriber = self.pd_alerts[i].dyn_subscriber().ok()?;
            Some((publisher, subscriber))
        });

        // If any subscriber creation failed, return None
        if pd_alerts.iter().any(|x| x.is_none()) {
            return None;
        }

        // Unwrap all elements (safe because we checked above)
        let mut iter = pd_alerts.into_iter();
        let pd_alerts = from_fn(|_| iter.next().unwrap().unwrap());

        Some(BackingDefault { pd_alerts })
    }
}

impl<const N: usize, M: RawMutex> Default for BackingDefaultStorage<N, M> {
    fn default() -> Self {
        Self::new()
    }
}

/// A reference to the storage provided by [`BackingDefaultStorage`].
pub struct BackingDefault<'a, const N: usize> {
    /// PD alert channels for each port
    pd_alerts: [(DynImmediatePublisher<'a, Ado>, DynSubscriber<'a, Ado>); N],
}

impl<'a, const N: usize> Backing<'a> for BackingDefault<'a, N> {
    async fn pd_alert_channel(
        &self,
        port_index: usize,
    ) -> Option<impl Deref<Target = (DynImmediatePublisher<'a, Ado>, DynSubscriber<'a, Ado>)>> {
        if port_index < N {
            Some(&self.pd_alerts[port_index])
        } else {
            None
        }
    }

    async fn pd_alert_channel_mut(
        &mut self,
        port_index: usize,
    ) -> Option<impl DerefMut<Target = (DynImmediatePublisher<'a, Ado>, DynSubscriber<'a, Ado>)>> {
        if port_index < N {
            Some(&mut self.pd_alerts[port_index])
        } else {
            None
        }
    }
}
