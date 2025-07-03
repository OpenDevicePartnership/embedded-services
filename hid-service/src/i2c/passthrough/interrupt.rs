use embassy_sync::{mutex::Mutex, signal::Signal};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::digital::Wait;
use embedded_services::{GlobalRawMutex, trace};

/// This struct manages interrupt signal passthrough
/// When an interrupt from the device occurs the interrupt to the host is assert
/// The interrupt will be deasserted when we receive a request from the host
/// We then ignore any further device interrupts until the response is sent to the host
pub struct InterruptSignal<IN: Wait, OUT: OutputPin> {
    state: Mutex<GlobalRawMutex, InterruptState>,
    int_in: Mutex<GlobalRawMutex, IN>,
    int_out: Mutex<GlobalRawMutex, OUT>,
    signal: Signal<GlobalRawMutex, ()>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InterruptState {
    Idle,
    Asserted,
    Waiting,
    Reset,
}

impl<IN: Wait, OUT: OutputPin> InterruptSignal<IN, OUT> {
    pub fn new(int_in: IN, int_out: OUT) -> Self {
        Self {
            state: Mutex::new(InterruptState::Idle),
            int_in: Mutex::new(int_in),
            int_out: Mutex::new(int_out),
            signal: Signal::new(),
        }
    }

    /// Deassert the interrupt signal
    pub async fn deassert(&self) {
        let mut state = self.state.lock().await;
        if *state == InterruptState::Asserted {
            *state = InterruptState::Waiting;
            self.signal.signal(());
        }
    }

    /// Release the interrupt signal, allowing device interrupts to passthrough again
    pub async fn release(&self) {
        let mut state = self.state.lock().await;
        if *state == InterruptState::Waiting {
            *state = InterruptState::Idle;
            self.signal.signal(());
        }
    }

    /// Deassert and release the interrupt signal
    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        *state = InterruptState::Reset;
        self.signal.signal(());
    }

    pub async fn process(&self) {
        let mut int_in = self.int_in.lock().await;
        let mut int_out = self.int_out.lock().await;

        trace!("Waiting for interrupt");

        int_in.wait_for_low().await.unwrap();

        int_out.set_low().unwrap();
        {
            let mut state = self.state.lock().await;
            *state = InterruptState::Asserted;
        }
        trace!("Interrupt received");

        self.signal.wait().await;
        int_out.set_high().unwrap();
        trace!("Interrupt deasserted");

        {
            let mut state = self.state.lock().await;
            if *state == InterruptState::Reset {
                *state = InterruptState::Idle;
                return;
            }
        }

        self.signal.wait().await;

        {
            let mut state = self.state.lock().await;
            *state = InterruptState::Idle;
        }
        trace!("Interrupt cleared");
    }
}
