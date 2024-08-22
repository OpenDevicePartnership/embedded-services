//! activity (dynamic) service definitions

use crate::Service;

/// potential activity service states
#[derive(Copy, Clone, Debug)]
pub enum State {
    /// the service is currently active
    Active,

    /// the service is currently in-active, but could become active
    Inactive,

    /// the service is disabled and will not become active
    Disabled,
}

/// specifies OEM identifier for extended activity services
pub type OemIdentifier = u32;

/// specifies which Activity Class is updating state
#[derive(Copy, Clone, Debug)]
pub enum Class {
    /// the keyboard, if present, is currently active (keys pressed), inactive (keys released), or disabled (key scanning disabled)
    Keyboard,

    /// the trackpad, if present, is currently active (swiped), inactive (no swiped), or disabled (powered off/unavailable)
    Trackpad,

    // SecureUpdate, others as needed for ec template
    /// OEM Extension class, for activity notifications that are OEM specific
    Oem(OemIdentifier),
}

/// notification datagram, containing who's activity state (class) changed and what the new state is
#[derive(Copy, Clone, Debug)]
pub struct Notification {
    /// activity state of this class
    pub state: State,

    /// classification of activity
    pub class: Class,
}

/// primary service instance
pub struct Manager {}

/// service configuration, if any (TODO Oem Limitations, for example)
pub struct Config {}

impl Service for Manager {
    type Notification = Notification;
    type Config = Config;

    fn init(_config: Self::Config) -> Self {
        Self {}
    }
}
