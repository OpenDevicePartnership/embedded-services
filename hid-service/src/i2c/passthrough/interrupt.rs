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
    int_out: Mutex<GlobalRawMutex, OutputState<OUT>>,
    signal: Signal<GlobalRawMutex, ()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Failed to read incoming interrupt line
    IoRead,
    /// Failed to assert/deassert outgoing interrupt line
    IoSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum InterruptState {
    Idle,
    Asserted,
    Waiting,
    Reset,
}

// Keep the successful physical level with its pin owner. The interrupt task
// must not repeat a fallible HIGH write after the host has confirmed it.
struct OutputState<OUT> {
    pin: OUT,
    high: bool,
}

impl<OUT: OutputPin> OutputState<OUT> {
    fn assert(&mut self) -> Result<(), Error> {
        // A failed write must also invalidate any previous HIGH confirmation.
        self.high = false;
        self.pin.set_low().map_err(|_| Error::IoSet)
    }

    fn deassert(&mut self) -> Result<(), Error> {
        if !self.high {
            self.pin.set_high().map_err(|_| Error::IoSet)?;
            self.high = true;
        }
        Ok(())
    }
}

impl<IN: Wait, OUT: OutputPin> InterruptSignal<IN, OUT> {
    pub const fn new(int_in: IN, int_out: OUT) -> Self {
        Self {
            state: Mutex::new(InterruptState::Idle),
            int_in: Mutex::new(int_in),
            int_out: Mutex::new(OutputState {
                pin: int_out,
                high: false,
            }),
            signal: Signal::new(),
        }
    }

    /// Request deassertion of the forwarded interrupt.
    ///
    /// Updates state and notifies [`Self::process`]; the output pin changes when
    /// that task runs, not necessarily before this future returns.
    ///
    /// Retained for one more release cycle to let existing callers migrate to
    /// [`Self::deassert_and_wait`], which confirms the pin transition and reports
    /// GPIO errors. This compatibility API keeps its notification-only behavior.
    #[deprecated(note = "Use deassert_and_wait() to confirm pin deassertion and handle GPIO errors")]
    pub async fn deassert(&self) {
        let mut state = self.state.lock().await;
        if *state == InterruptState::Asserted {
            *state = InterruptState::Waiting;
            self.signal.signal(());
        }
    }

    /// Drive the forwarded interrupt high before returning successfully.
    ///
    /// Performs the GPIO write directly rather than waiting for [`Self::process`]
    /// to run. The state transition is published only after a successful write;
    /// on failure the caller must not proceed with its host response.
    pub async fn deassert_and_wait(&self) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        self.int_out.lock().await.deassert()?;

        if *state == InterruptState::Asserted {
            *state = InterruptState::Waiting;
            self.signal.signal(());
        }

        Ok(())
    }

    /// Mark the request complete, allowing device interrupts to pass through again.
    pub async fn release(&self) {
        let mut state = self.state.lock().await;
        if *state == InterruptState::Waiting {
            *state = InterruptState::Idle;
            self.signal.signal(());
        }
    }

    /// Request deassertion and release of an active interrupt.
    ///
    /// Idle and already-reset exchanges need no notification. The output-pin
    /// transition is performed by [`Self::process`] when that task runs.
    pub async fn reset(&self) {
        let mut state = self.state.lock().await;

        // Only an active interrupt exchange needs waking. An idle notification
        // would be consumed by the next interrupt and deassert it prematurely.
        // Reset is already notified, so repeated resets need no extra signal.
        if matches!(*state, InterruptState::Asserted | InterruptState::Waiting) {
            *state = InterruptState::Reset;
            self.signal.signal(());
        }
    }

    pub async fn process(&self) -> Result<(), Error> {
        let mut int_in = self.int_in.lock().await;

        trace!("Waiting for interrupt");

        int_in.wait_for_low().await.map_err(|_| Error::IoRead)?;

        {
            let mut state = self.state.lock().await;
            self.int_out.lock().await.assert()?;
            *state = InterruptState::Asserted;
        }
        trace!("Interrupt received");

        self.signal.wait().await;
        self.int_out.lock().await.deassert()?;
        trace!("Interrupt deasserted");

        {
            let mut state = self.state.lock().await;
            // Deassert and release may both finish before this task resumes.
            // Signal coalesces their notifications, but Idle records that the
            // release already happened, so do not wait for a second signal.
            if matches!(*state, InterruptState::Reset | InterruptState::Idle) {
                // Producers also hold state while signaling. Retire a second
                // notification posted after our first wait consumed its value,
                // before making the next interrupt exchange possible.
                self.signal.reset();
                *state = InterruptState::Idle;
                return Ok(());
            }
        }

        self.signal.wait().await;

        {
            let mut state = self.state.lock().await;
            *state = InterruptState::Idle;
        }
        trace!("Interrupt cleared");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Retain coverage of the deprecated notification-only behavior.
    #![allow(deprecated)]
    #![allow(clippy::unwrap_used)]

    extern crate std;

    use core::convert::Infallible;
    use core::future::{Future, pending};
    use core::pin::{Pin, pin};
    use core::task::{Context, Poll, Waker};

    use embassy_futures::block_on;
    use embedded_hal::digital::ErrorType;
    use std::{
        boxed::Box,
        cell::{Cell, RefCell},
        rc::Rc,
        vec::Vec,
    };

    use super::*;

    type Edges = Rc<RefCell<Vec<bool>>>;
    type OnHigh = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

    struct Input;

    impl ErrorType for Input {
        type Error = Infallible;
    }

    impl Wait for Input {
        async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
            pending().await
        }

        async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
            pending().await
        }

        async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
            pending().await
        }

        async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
            pending().await
        }
    }

    struct Output {
        edges: Edges,
        on_high: Option<OnHigh>,
    }

    impl ErrorType for Output {
        type Error = Infallible;
    }

    impl OutputPin for Output {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            self.edges.as_ref().borrow_mut().push(false);
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            self.edges.as_ref().borrow_mut().push(true);

            if let Some(hook) = &self.on_high {
                let callback = hook.as_ref().borrow_mut().take();

                if let Some(callback) = callback {
                    callback();
                }
            }

            Ok(())
        }
    }

    fn signal() -> (InterruptSignal<Input, Output>, Edges) {
        signal_with_hook(None)
    }

    fn signal_with_hook(on_high: Option<OnHigh>) -> (InterruptSignal<Input, Output>, Edges) {
        let edges = Rc::new(RefCell::new(Vec::new()));
        let output = Output {
            edges: edges.clone(),
            on_high,
        };
        let signal = InterruptSignal::new(Input, output);

        (signal, edges)
    }

    fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn confirmed_deassertion_drives_pin_without_rescheduling_interrupt_task() {
        let (signal, edges) = signal();

        for _ in 0..2 {
            let mut processing = pin!(signal.process());
            // Assert LOW, then stop at the first self.signal.wait().
            assert!(matches!(poll(processing.as_mut()), Poll::Pending));
            assert_eq!(edges.as_ref().borrow().last(), Some(&false));

            block_on(signal.deassert_and_wait()).unwrap();
            assert_eq!(edges.as_ref().borrow().last(), Some(&true));

            block_on(signal.release());
            // Consume the coalesced notifications; Idle skips the second wait.
            assert!(matches!(poll(processing.as_mut()), Poll::Ready(Ok(()))));
        }
    }

    #[test]
    fn confirmed_deassertion_is_not_rewritten_while_the_response_is_pending() {
        struct FailAfterFirstHigh(Rc<Cell<bool>>);

        impl ErrorType for FailAfterFirstHigh {
            type Error = embedded_hal::digital::ErrorKind;
        }

        impl OutputPin for FailAfterFirstHigh {
            fn set_low(&mut self) -> Result<(), Self::Error> {
                self.0.set(false);
                Ok(())
            }

            fn set_high(&mut self) -> Result<(), Self::Error> {
                if self.0.replace(true) {
                    return Err(embedded_hal::digital::ErrorKind::Other);
                }
                Ok(())
            }
        }

        let high = Rc::new(Cell::new(true));
        let signal = InterruptSignal::new(Input, FailAfterFirstHigh(high.clone()));
        let mut processing = pin!(signal.process());
        // Assert LOW, then stop at the first self.signal.wait().
        assert!(matches!(poll(processing.as_mut()), Poll::Pending));
        block_on(signal.deassert_and_wait()).unwrap();
        assert!(high.get());

        // The host may be waiting for its response while the interrupt task
        // resumes. A redundant, fallible HIGH write must not restart this
        // exchange and reassert LOW underneath that pending response.
        // Consume deassertion, skip the confirmed HIGH write, and reach the second wait.
        assert!(matches!(poll(processing.as_mut()), Poll::Pending));
        assert!(high.get());
        block_on(signal.release());
        // Consume release at the second self.signal.wait() and complete.
        assert!(matches!(poll(processing.as_mut()), Poll::Ready(Ok(()))));
    }

    #[test]
    fn confirmed_deassertion_while_idle_does_not_notify_the_next_interrupt() {
        let (signal, edges) = signal();
        block_on(signal.deassert_and_wait()).unwrap();
        assert_eq!(edges.as_ref().borrow().last(), Some(&true));

        let mut processing = pin!(signal.process());
        // Assert LOW and reach the first wait; idle deassertion left no notification.
        assert!(matches!(poll(processing.as_mut()), Poll::Pending));
        assert_eq!(edges.as_ref().borrow().last(), Some(&false));

        block_on(signal.deassert_and_wait()).unwrap();
        assert_eq!(edges.as_ref().borrow().last(), Some(&true));
        // Consume deassertion, skip the confirmed HIGH write, and reach the second wait.
        assert!(matches!(poll(processing.as_mut()), Poll::Pending));
        block_on(signal.release());
        // Consume release at the second self.signal.wait() and complete.
        assert!(matches!(poll(processing.as_mut()), Poll::Ready(Ok(()))));
    }

    #[test]
    fn failed_confirmed_deassertion_can_be_retried_without_releasing_the_exchange() {
        struct FallibleOutput {
            fail: Rc<Cell<bool>>,
            high: Rc<Cell<bool>>,
        }

        impl ErrorType for FallibleOutput {
            type Error = embedded_hal::digital::ErrorKind;
        }

        impl OutputPin for FallibleOutput {
            fn set_low(&mut self) -> Result<(), Self::Error> {
                self.high.set(false);
                Ok(())
            }

            fn set_high(&mut self) -> Result<(), Self::Error> {
                if self.fail.get() {
                    return Err(embedded_hal::digital::ErrorKind::Other);
                }
                self.high.set(true);
                Ok(())
            }
        }

        let fail = Rc::new(Cell::new(true));
        let high = Rc::new(Cell::new(true));
        let signal = InterruptSignal::new(
            Input,
            FallibleOutput {
                fail: fail.clone(),
                high: high.clone(),
            },
        );
        let mut processing = pin!(signal.process());
        // Assert LOW, then stop at the first self.signal.wait().
        assert!(matches!(poll(processing.as_mut()), Poll::Pending));
        assert!(!high.get());

        assert_eq!(block_on(signal.deassert_and_wait()), Err(Error::IoSet));
        assert!(!high.get());
        // The failed GPIO write did not notify process; stay at the first wait.
        assert!(matches!(poll(processing.as_mut()), Poll::Pending));

        fail.set(false);
        block_on(signal.deassert_and_wait()).unwrap();
        assert!(high.get());
        // Consume deassertion, skip the confirmed HIGH write, and reach the second wait.
        assert!(matches!(poll(processing.as_mut()), Poll::Pending));
        block_on(signal.release());
        // Consume release at the second self.signal.wait() and complete.
        assert!(matches!(poll(processing.as_mut()), Poll::Ready(Ok(()))));
    }

    #[test]
    fn immediate_completion_does_not_lose_interrupt_release() {
        let (signal, edges) = signal();

        for _ in 0..2 {
            let mut processing = pin!(signal.process());
            // Assert LOW, then stop at the first self.signal.wait().
            assert!(matches!(poll(processing.as_mut()), Poll::Pending));

            // A fast command can deassert and finish before the interrupt task
            // runs again. Both notifications then share one Signal slot.
            block_on(signal.deassert());
            block_on(signal.release());

            // Consume the coalesced notification, drive HIGH, and skip the second wait (Idle).
            assert!(matches!(poll(processing.as_mut()), Poll::Ready(Ok(()))));
        }

        assert_eq!(*edges.as_ref().borrow(), [false, true, false, true]);
    }

    #[test]
    fn concurrent_completion_cannot_leave_a_notification_for_next_interrupt() {
        for reset in [false, true] {
            let hook: OnHigh = Rc::new(RefCell::new(None));
            let (signal, edges) = signal_with_hook(Some(hook.clone()));
            let signal = &*Box::leak(Box::new(signal));

            // Inject a host transition at the pin-write boundary, after the
            // first notification is consumed but before process checks state.
            // This deterministically models a concurrent executor interleaving.
            *hook.as_ref().borrow_mut() = Some(Box::new(move || {
                if reset {
                    block_on(signal.reset());
                } else {
                    block_on(signal.release());
                }
            }));

            {
                let mut processing = pin!(signal.process());
                // Assert LOW, then stop at the first self.signal.wait().
                assert!(matches!(poll(processing.as_mut()), Poll::Pending));

                block_on(signal.deassert());
                // Consume deassertion and drive HIGH, invoking the hook. Idle/Reset
                // then skips the second wait and clears the hook's notification.
                assert!(matches!(poll(processing.as_mut()), Poll::Ready(Ok(()))));
            }

            let mut next = pin!(signal.process());
            // Assert LOW and reach the first wait, with no stale notification.
            assert!(matches!(poll(next.as_mut()), Poll::Pending));
            assert_eq!(*edges.as_ref().borrow(), [false, true, false]);

            block_on(signal.deassert());
            // Consume deassertion, drive HIGH, and stop at the second wait.
            assert!(matches!(poll(next.as_mut()), Poll::Pending));

            block_on(signal.release());
            // Consume release at the second self.signal.wait() and complete.
            assert!(matches!(poll(next.as_mut()), Poll::Ready(Ok(()))));
            assert_eq!(*edges.as_ref().borrow(), [false, true, false, true]);
        }
    }

    #[test]
    fn reset_while_idle_does_not_deassert_the_next_interrupt() {
        let (signal, edges) = signal();
        block_on(signal.reset());

        let mut next = pin!(signal.process());
        // Assert LOW and reach the first wait; idle reset left no notification.
        assert!(matches!(poll(next.as_mut()), Poll::Pending));
        assert_eq!(*edges.as_ref().borrow(), [false]);

        block_on(signal.deassert());
        // Consume deassertion, drive HIGH, and stop at the second wait.
        assert!(matches!(poll(next.as_mut()), Poll::Pending));

        block_on(signal.release());
        // Consume release at the second self.signal.wait() and complete.
        assert!(matches!(poll(next.as_mut()), Poll::Ready(Ok(()))));
        assert_eq!(*edges.as_ref().borrow(), [false, true]);
    }

    #[test]
    fn reset_allows_next_interrupt_at_each_completion_phase() {
        for phase in 0..3 {
            let (signal, edges) = signal();

            {
                let mut processing = pin!(signal.process());
                // Assert LOW, then stop at the first self.signal.wait().
                assert!(matches!(poll(processing.as_mut()), Poll::Pending));

                if phase >= 1 {
                    block_on(signal.deassert());
                }

                if phase == 2 {
                    // Consume deassertion, drive HIGH, and stop at the second wait.
                    assert!(matches!(poll(processing.as_mut()), Poll::Pending));
                }

                block_on(signal.reset());
                // Phases 0/1 consume reset at the first wait, drive HIGH, and
                // skip the second wait. Phase 2 consumes reset at the second wait.
                assert!(matches!(poll(processing.as_mut()), Poll::Ready(Ok(()))));
            }

            let mut next = pin!(signal.process());
            // Assert LOW and reach the first wait, with no stale reset notification.
            assert!(matches!(poll(next.as_mut()), Poll::Pending));

            block_on(signal.deassert());
            // Consume deassertion, drive HIGH, and stop at the second wait.
            assert!(matches!(poll(next.as_mut()), Poll::Pending));

            block_on(signal.release());
            // Consume release at the second self.signal.wait() and complete.
            assert!(matches!(poll(next.as_mut()), Poll::Ready(Ok(()))));
            assert_eq!(*edges.as_ref().borrow(), [false, true, false, true]);
        }
    }
}
