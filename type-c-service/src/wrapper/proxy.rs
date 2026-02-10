use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use power_policy_service::psu::{CommandData as PolicyCommandData, InternalResponseData as PolicyResponseData, Psu};

pub struct PowerProxyChannel<M: RawMutex> {
    command_channel: Channel<M, PolicyCommandData, 1>,
    response_channel: Channel<M, PolicyResponseData, 1>,
}

impl<M: RawMutex> PowerProxyChannel<M> {
    pub fn new() -> Self {
        Self {
            command_channel: Channel::new(),
            response_channel: Channel::new(),
        }
    }

    pub fn get_device(&self) -> PowerProxyDevice<'_> {
        PowerProxyDevice {
            sender: self.command_channel.dyn_sender(),
            receiver: self.response_channel.dyn_receiver(),
        }
    }

    pub fn get_receiver(&self) -> PowerProxyReceiver<'_> {
        PowerProxyReceiver {
            receiver: self.command_channel.dyn_receiver(),
            sender: self.response_channel.dyn_sender(),
        }
    }
}

pub struct PowerProxyReceiver<'a> {
    sender: DynamicSender<'a, PolicyResponseData>,
    receiver: DynamicReceiver<'a, PolicyCommandData>,
}

impl<'a> PowerProxyReceiver<'a> {
    pub fn new(
        receiver: DynamicReceiver<'a, PolicyCommandData>,
        sender: DynamicSender<'a, PolicyResponseData>,
    ) -> Self {
        Self { receiver, sender }
    }

    pub async fn receive(&mut self) -> PolicyCommandData {
        self.receiver.receive().await
    }

    pub async fn send(&mut self, response: PolicyResponseData) {
        self.sender.send(response).await;
    }
}

pub struct PowerProxyDevice<'a> {
    sender: DynamicSender<'a, PolicyCommandData>,
    receiver: DynamicReceiver<'a, PolicyResponseData>,
}

impl<'a> PowerProxyDevice<'a> {
    pub fn new(
        sender: DynamicSender<'a, PolicyCommandData>,
        receiver: DynamicReceiver<'a, PolicyResponseData>,
    ) -> Self {
        Self { sender, receiver }
    }

    async fn execute(&mut self, command: PolicyCommandData) -> PolicyResponseData {
        self.sender.send(command).await;
        self.receiver.receive().await
    }
}

impl<'a> Psu for PowerProxyDevice<'a> {
    async fn disconnect(&mut self) -> Result<(), power_policy_service::psu::Error> {
        self.execute(PolicyCommandData::Disconnect).await?.complete_or_err()
    }

    async fn connect_provider(
        &mut self,
        capability: power_policy_service::capability::ProviderPowerCapability,
    ) -> Result<(), power_policy_service::psu::Error> {
        self.execute(PolicyCommandData::ConnectAsProvider(capability))
            .await?
            .complete_or_err()
    }

    async fn connect_consumer(
        &mut self,
        capability: power_policy_service::capability::ConsumerPowerCapability,
    ) -> Result<(), power_policy_service::psu::Error> {
        self.execute(PolicyCommandData::ConnectAsConsumer(capability))
            .await?
            .complete_or_err()
    }
}

impl<M: RawMutex> Default for PowerProxyChannel<M> {
    fn default() -> Self {
        Self::new()
    }
}
