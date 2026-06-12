use power_policy_interface::psu::Error;

use crate::service::{
    InternalState,
    config::Config,
    consumer::{AvailableConsumer, cmp_consumer_capability_default, find_best_consumer_default},
    registration::Registration,
};

/// Power policy service hooks
pub trait Hooks {
    /// Find the best available consumer based on the current state and configuration.
    fn find_best_consumer<'device, Reg: Registration<'device>>(
        &mut self,
        config: &Config,
        state: &InternalState<'device, Reg::Psu>,
        registration: &Reg,
    ) -> impl Future<Output = Result<Option<AvailableConsumer<'device, Reg::Psu>>, Error>> {
        find_best_consumer_default(config, state, registration, cmp_consumer_capability_default)
    }
}

/// Default hooks implementation
#[derive(Debug, Clone, Default)]
pub struct DefaultHooks;

impl Hooks for DefaultHooks {}
