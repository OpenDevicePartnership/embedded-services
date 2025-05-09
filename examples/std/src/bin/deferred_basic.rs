//! Basic example of using a deferred channel
//! This demonstrates inoking commands from multiple tasks
//! and shows correct handling when a response is not read due to a timeout
use embassy_executor::{Executor, Spawner};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::{with_timeout, Duration, Timer};
use embedded_services::ipc::deferred;
use log::*;
use static_cell::StaticCell;

/// Mock commands
#[derive(Debug)]
enum Command {
    A,
    B,
    C,
}

/// Mock responses
#[derive(Debug)]
enum Response {
    A,
    B,
    C,
}

/// Mock command handler
struct Handler {
    channel: deferred::Channel<NoopRawMutex, Command, Response>,
}

impl Handler {
    /// Create a new handler
    fn new() -> Self {
        Self {
            channel: deferred::Channel::new(),
        }
    }

    /// Process a command and return a response
    async fn process_request(&self, request: &Command) -> Response {
        match request {
            Command::A => {
                info!("Processing request A");
                Response::A
            }
            Command::B => {
                info!("Processing request B");
                Response::B
            }
            Command::C => {
                info!("Processing request C");
                // Request that takes a while to finish
                Timer::after_millis(1000).await;
                Response::C
            }
        }
    }

    /// Invoke command A
    async fn invoke_a(&self) -> Response {
        info!("Requesting A");
        self.channel.invoke(Command::A).await
    }

    /// Invoke command B
    async fn invoke_b(&self) -> Response {
        info!("Requesting B");
        self.channel.invoke(Command::B).await
    }

    /// Invoke command C
    async fn invoke_c(&self) -> Response {
        info!("Requesting C");
        self.channel.invoke(Command::C).await
    }

    /// Main processing task
    async fn process(&self) {
        loop {
            let invocation = self.channel.wait_invocation().await;
            info!("Received command: {:?}", invocation.command);
            let response = self.process_request(&invocation.command).await;
            invocation.send_response(response);
        }
    }
}

/// Task that executes command A
#[embassy_executor::task]
async fn invoker_task_0(handler: &'static Handler) {
    info!("Invoker task 0 started");
    info!("Invoking C");
    let response = with_timeout(Duration::from_millis(250), handler.invoke_c()).await;
    info!("Invoker task 0 received response: {:?}", response);
    info!("Invoking A");
    let response = handler.invoke_a().await;
    info!("Invoker task 0 received response: {:?}", response);
}

/// Task that executes command B
#[embassy_executor::task]
async fn invoker_task_1(handler: &'static Handler) {
    info!("Invoker task 1 started");
    let response = handler.invoke_b().await;
    info!("Invoker task 1 received response: {:?}", response);
}

/// Task that handles device commands
#[embassy_executor::task]
async fn handler_task(handler: &'static Handler) {
    info!("Handler task started");
    loop {
        handler.process().await;
    }
}

/// Main task that initializes the device and spawns other tasks
#[embassy_executor::task]
async fn task(spawner: Spawner) {
    static DEVICE: OnceLock<Handler> = OnceLock::new();

    let device = DEVICE.get_or_init(Handler::new);
    spawner.must_spawn(handler_task(device));
    spawner.must_spawn(invoker_task_0(device));
    spawner.must_spawn(invoker_task_1(device));
}

/// Entry point
fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(task(spawner)).unwrap();
    });
}
