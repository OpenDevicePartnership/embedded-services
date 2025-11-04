use embedded_services::info;
use embedded_services::power::policy::device::DeviceTrait;
use embedded_services::power::policy::flags::Consumer;
use embedded_services::power::policy::policy::Sender;
use embedded_services::power::policy::{ConsumerPowerCapability, Error, PowerCapability, ProviderPowerCapability};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FnCall {
    ConnectConsumer(ConsumerPowerCapability),
    ConnectProvider(ProviderPowerCapability),
    Disconnect,
    Reset,
}

pub struct Mock<S: Sender> {
    sender: S,
    // Number of function calls made to the mock.
    pub num_fn_calls: usize,
    // Last function call made.
    pub last_fn_call: Option<FnCall>,
}

impl<S: Sender> Mock<S> {
    pub fn new(sender: S) -> Self {
        Self {
            sender,
            num_fn_calls: 0,
            last_fn_call: None,
        }
    }

    pub async fn simulate_consumer_connection(&mut self, capability: PowerCapability) {
        self.sender.on_attach().await;
        self.sender
            .on_update_consumer_capability(Some(ConsumerPowerCapability {
                capability,
                flags: Consumer::none(),
            }))
            .await;
    }

    #[allow(dead_code)]
    pub async fn simulate_detach(&mut self) {
        self.sender.on_detach().await;
    }

    pub fn reset_mock(&mut self) {
        self.num_fn_calls = 0;
        self.last_fn_call = None;
    }
}

impl<S: Sender> DeviceTrait for Mock<S> {
    async fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), Error> {
        info!("Connect consumer {:#?}", capability);
        self.num_fn_calls += 1;
        self.last_fn_call = Some(FnCall::ConnectConsumer(capability));
        Ok(())
    }

    async fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), Error> {
        info!("Connect provider: {:#?}", capability);
        self.num_fn_calls += 1;
        self.last_fn_call = Some(FnCall::ConnectProvider(capability));
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), Error> {
        info!("Disconnect");
        self.num_fn_calls += 1;
        self.last_fn_call = Some(FnCall::Disconnect);
        Ok(())
    }

    async fn reset(&mut self) -> Result<(), Error> {
        info!("Reset");
        self.num_fn_calls += 1;
        self.last_fn_call = Some(FnCall::Reset);
        Ok(())
    }
}
