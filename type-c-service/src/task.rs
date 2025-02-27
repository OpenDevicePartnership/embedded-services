use core::cell::RefCell;

use bitfield::Bit;
use embassy_sync::once_lock::OnceLock;
use embedded_services::{
    debug, error, info,
    type_c::{self, controller::PortStatus, GlobalPortId},
};
use embedded_usb_pd::PdError as Error;

const MAX_SUPPORTED_PORTS: usize = 4;

/// Type-C service state
#[derive(Default)]
struct State {
    /// Current port status
    port_status: [PortStatus; MAX_SUPPORTED_PORTS],
}

/// Type-C service
struct Service {
    /// Type-C context token
    context: type_c::controller::ContextToken,
    /// Current state
    state: RefCell<State>,
}

impl Service {
    /// Create a new service
    pub fn create() -> Option<Self> {
        Some(Self {
            context: type_c::controller::ContextToken::create()?,
            state: RefCell::new(State::default()),
        })
    }

    /// Get the cached port status
    fn get_cached_port_status(&self, port_id: GlobalPortId) -> Result<PortStatus, Error> {
        if port_id.0 as usize >= MAX_SUPPORTED_PORTS {
            return Err(Error::InvalidPort);
        }

        let state = self.state.borrow();
        Ok(state.port_status[port_id.0 as usize])
    }

    /// Set the cached port status
    fn set_cached_port_status(&self, port_id: GlobalPortId, status: PortStatus) -> Result<(), Error> {
        if port_id.0 as usize >= MAX_SUPPORTED_PORTS {
            return Err(Error::InvalidPort);
        }

        let mut state = self.state.borrow_mut();
        state.port_status[port_id.0 as usize] = status;
        Ok(())
    }

    /// Process events for a specific port
    async fn process_port_events(&self, port_id: GlobalPortId) -> Result<(), Error> {
        let event = self.context.get_port_event(port_id).await?;
        let status = self.context.get_port_status(port_id).await?;
        let old_status = self.get_cached_port_status(port_id)?;

        debug!("Port{}: Event: {:#?}", port_id.0, event);
        debug!("Port{} Previous status: {:#?}", port_id.0, old_status);
        debug!("Port{} Status: {:#?}", port_id.0, status);

        self.set_cached_port_status(port_id, status)?;

        Ok(())
    }

    /// Main processing function
    pub async fn process(&self) {
        let events = self.context.get_unhandled_events().await;

        for i in 0..events.len() {
            let port_id = GlobalPortId(i as u8);

            if !events.bit(i) {
                continue;
            }

            debug!("Port{}: Event", i);
            if let Err(e) = self.process_port_events(port_id).await {
                error!("Port{}: Error processing events: {:#?}", i, e);
            }
        }
    }
}

#[embassy_executor::task]
pub async fn task() {
    info!("Starting type-c task");

    let service = Service::create();
    let service = match service {
        Some(service) => service,
        None => {
            error!("Type-C service already initialized");
            return;
        }
    };

    static SERVICE: OnceLock<Service> = OnceLock::new();
    let service = SERVICE.get_or_init(|| service);

    loop {
        service.process().await;
    }
}
