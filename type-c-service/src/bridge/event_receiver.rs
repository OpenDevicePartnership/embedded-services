use embedded_services::{GlobalRawMutex, ipc::deferred};
use type_c_interface::port;

pub type ControllerCommand<'a> = deferred::Request<'a, GlobalRawMutex, port::Command, port::Response<'static>>;

/// Controller command output data
pub struct OutputControllerCommand<'a> {
    /// Controller request
    pub request: ControllerCommand<'a>,
    /// Response
    pub response: port::Response<'static>,
}

pub struct EventReceiver {
    /// PD controller
    pub pd_controller: &'static port::Device<'static>,
}

impl EventReceiver {
    /// Create a new instance
    pub fn new(pd_controller: &'static port::Device<'static>) -> Self {
        Self { pd_controller }
    }

    pub fn wait_next(&mut self) -> impl Future<Output = ControllerCommand<'static>> {
        self.pd_controller.receive()
    }

    pub fn finalize(&mut self, output: OutputControllerCommand<'static>) {
        output.request.respond(output.response);
    }
}
