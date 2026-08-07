//! A configurable GPIO matrix keyboard service.
//!
//! [`Service`] implements the transport-neutral [`KeyboardService`] interface and
//! [`odp_service_common::runnable_service::Service`]. Use [`crate::KeyboardHidRelay`] to expose it
//! through HID, or provide another relay implementation for a different protocol.
use core::cell::Cell;
use core::marker::PhantomData;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::pubsub::{DynSubscriber, PubSubChannel, Publisher};
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_services::{GlobalRawMutex, Never, error, warn};
use keyberon::debounce::Debouncer;
use keyberon::key_code::KbHidReport;
use keyberon::layout::Layout;
pub use keyberon::layout::{Layers, layout};
use keyberon::matrix::Matrix;

use crate::interface::{KeyboardInputReport, KeyboardPowerState, KeyboardService, LedFlags};

// Depth of the channel carrying input reports from the scan `Runner` to subscribers.
const REPORT_QUEUE_DEPTH: usize = 8;

// HID keyboard error codes placed in the key slots of an input report to signal an error condition.
const ERROR_ROLL_OVER: u8 = 0x01;
const ERROR_UNDEFINED: u8 = 0x03;

/// Keyboard service error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyboardError {
    /// Failed to drive a GPIO (e.g. a row/column pin or an LED pin).
    Scan,
}

/// Error returned while initializing a GPIO keyboard service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyboardInitError<E> {
    /// Failed to initialize the key matrix.
    Matrix(E),
    /// Deghosting supports at most 128 rows.
    TooManyRowsForDeghosting,
}

fn set_led(led: &mut Option<impl OutputPin>, cond: bool) -> Result<(), KeyboardError> {
    if let Some(led) = led {
        if cond {
            led.set_high().map_err(|_| KeyboardError::Scan)?;
        } else {
            led.set_low().map_err(|_| KeyboardError::Scan)?;
        }
    }

    Ok(())
}

/// State shared between the [`Service`] control handle and its scanning [`Runner`].
struct Shared<const SUBS: usize> {
    /// Input reports produced by the scan `Runner` and consumed by the control handle.
    reports: PubSubChannel<GlobalRawMutex, KeyboardInputReport, REPORT_QUEUE_DEPTH, SUBS, 1>,
    /// Current host-commanded power state, written by the control handle and read by the scanner.
    power_state: BlockingMutex<GlobalRawMutex, Cell<KeyboardPowerState>>,
    /// Pulsed whenever `power_state` changes so the scanner can re-evaluate its power gate.
    power_changed: Signal<GlobalRawMutex, ()>,
}

impl<const SUBS: usize> Shared<SUBS> {
    fn new() -> Self {
        Self {
            reports: PubSubChannel::new(),
            // Spec says a device starts in the ON state, but we gate scanning until the host
            // explicitly powers us on, matching the previous keyboard-service behavior.
            power_state: BlockingMutex::new(Cell::new(KeyboardPowerState::Sleep)),
            power_changed: Signal::new(),
        }
    }

    fn is_powered_on(&self) -> bool {
        self.power_state
            .lock(|state| matches!(state.get(), KeyboardPowerState::On))
    }

    fn set_power_state(&self, state: KeyboardPowerState) {
        self.power_state.lock(|cell| cell.set(state));
        self.power_changed.signal(());
    }
}

/// GPIO keyboard configuration.
pub struct KeyboardConfig<
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    DELAY: FnMut(),
> {
    /// An array of input pins representing each row.
    pub rows: [INPUT; NROWS],
    /// An array of output pins representing each column.
    pub cols: [OUTPUT; NCOLS],
    /// A keyberon layers implementation which maps coordinates to keys.
    pub layers: &'static Layers<NCOLS, NROWS, NLAYERS>,
    /// The interval in milliseconds between each scan.
    pub poll_ms: u64,
    /// The number of times an event (e.g. a key press) needs to be seen to actually register.
    pub nb_bounce: u16,
    /// A function that provides some blocking delay implementation.
    /// This is used during scan between driving a row and reading a column.
    pub delay: DELAY,
    /// If enabled, the scanner will perform ghosting detection,
    /// and report an error to host if detected.
    ///
    /// This will also discard false positives, so for a full NKRO/diode-per-switch keyboard,
    /// it is best to leave this disabled.
    pub deghost: bool,
}

// Internal keyberon configuration which the public KeyboardConfig gets converted to
struct KeyberonConfig<
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    DELAY: FnMut(),
> {
    matrix: Matrix<INPUT, OUTPUT, NROWS, NCOLS>,
    debouncer: Debouncer<[[bool; NROWS]; NCOLS]>,
    layout: Layout<NCOLS, NROWS, NLAYERS>,
    poll_ms: u64,
    delay: DELAY,
    deghost: bool,
}

impl<
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    DELAY: FnMut(),
> TryFrom<KeyboardConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>>
    for KeyberonConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>
{
    type Error = E;

    fn try_from(cfg: KeyboardConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>) -> Result<Self, E> {
        Ok(Self {
            // Keyberon expects colums as input and rows as output, but most platforms seem opposite?
            // So we swap them, and during scan perform a transform to reverse coordinates.
            //
            // Revisit: See if there is an easy way to support both formats generically
            matrix: Matrix::new(cfg.rows, cfg.cols)?,
            debouncer: keyberon::debounce::Debouncer::new(
                [[false; NROWS]; NCOLS],
                [[false; NROWS]; NCOLS],
                cfg.nb_bounce,
            ),
            layout: Layout::new(cfg.layers),
            poll_ms: cfg.poll_ms,
            delay: cfg.delay,
            deghost: cfg.deghost,
        })
    }
}

/// Keyboard LED configuration.
///
/// HID spec defines many usage IDs for LED page, so trying to support them here is difficult.
/// So it has been narrowed down to just 3 that may actually be common on modern laptop keyboards.
pub struct LedConfig<LED: OutputPin> {
    /// Num lock key LED if available.
    pub num_lock: Option<LED>,
    /// Caps lock key LED if available.
    pub caps_lock: Option<LED>,
    /// Scroll lock key LED if available.
    pub scroll_lock: Option<LED>,
}

fn has_ghost<const NROWS: usize, const NCOLS: usize>(pressed: &[[bool; NROWS]; NCOLS]) -> bool {
    // First convert rows represented as an array of bools into packed bits
    // This is likely more efficient than doing a triple nested loop below,
    // since this allows us to quickly check bits
    // Chose u128 as it's the largest primitive and it's very unlikely a keyboard will have more than 128 rows
    let mut pressed_bits = [0u128; NCOLS];
    let mut count = 0;
    for (row, pressed_bits_col) in pressed.iter().zip(pressed_bits.iter_mut()) {
        for (r, &key) in row.iter().enumerate() {
            if key {
                count += 1;
                *pressed_bits_col |= 1 << r;
            }
        }
    }

    // Ghosting is only possible when >2 keys are simultaneously pressed
    if count <= 2 {
        return false;
    }

    // Compare every column against every other column.
    //
    // If bitwise and between two columns has >= 2 bits set,
    // at least two pressed keys share same row and column,
    // which means a one of those keys reported as pressed is very likely a ghost.
    //
    // This can report false positives however, as the user might actually be pressing
    // 4 keys forming the corners of a rectangle. This is unlikely however, as keypads are typically
    // wired to make this improbable, so the usual response is to discard the input regardless
    // and report rollover error to the host.
    //
    // Also note this is sufficient only on a complete post-scan result.
    // There are tricks mid-scan to detect 3 keys in L-shape (which would cause ghost later on in the scan)
    // and bail early, but that would require modifiying keyberon.
    //
    // So we essentially complete a scan, check for ghosts, THEN pass into debouncer.
    for (i, c1) in pressed_bits.iter().enumerate() {
        for c2 in pressed_bits[i + 1..].iter() {
            if (c1 & c2).count_ones() >= 2 {
                return true;
            }
        }
    }

    false
}

/// Caller-allocated storage for a keyboard [`Service`].
///
/// `SUBS` is the maximum number of simultaneous input-report subscribers.
///
/// This is an opaque type; construct it with [`Default`] and hand a `&mut` reference to
/// [`Service::new`] (typically via `spawn_service!`).
pub struct Resources<const SUBS: usize> {
    inner: Option<Shared<SUBS>>,
}

impl<const SUBS: usize> Default for Resources<SUBS> {
    fn default() -> Self {
        Self { inner: None }
    }
}

/// The outcome of a single scan cycle in the [`Runner`].
enum ScanOutcome {
    /// A fresh input report is ready to be sent to the host.
    Report(KeyboardInputReport),
    /// The keyboard was powered down mid-cycle; the runner should re-gate on power.
    Unpowered,
}

/// Scanning runner for a GPIO keyboard [`Service`].
///
/// Owns the key matrix and drives it in a loop, pushing input reports to the control handle. You
/// must call [`run`](odp_service_common::runnable_service::ServiceRunner::run) on this to make the
/// keyboard produce reports; consider using `spawn_service!`.
pub struct Runner<
    'hw,
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    const SUBS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    DELAY: FnMut(),
> {
    shared: &'hw Shared<SUBS>,
    publisher: Publisher<'hw, GlobalRawMutex, KeyboardInputReport, REPORT_QUEUE_DEPTH, SUBS, 1>,
    kb: KeyberonConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>,
}

impl<
    'hw,
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    const SUBS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    DELAY: FnMut(),
> Runner<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, DELAY>
{
    /// Polls the matrix until a report is ready, an error is detected, or the keyboard is powered off.
    async fn scan_cycle(&mut self) -> ScanOutcome {
        loop {
            // Stop producing reports as soon as the host powers us down.
            if !self.shared.is_powered_on() {
                return ScanOutcome::Unpowered;
            }

            match self.kb.matrix.get_with_delay(&mut self.kb.delay) {
                Ok(pressed) => {
                    // If ghosting detected, report a rollover error.
                    if self.kb.deghost && has_ghost(&pressed) {
                        warn!("Key ghosting detected");
                        return ScanOutcome::Report(KeyboardInputReport::error(ERROR_ROLL_OVER));
                    }

                    // Run the scan through the debouncer, applying a coordinate transform.
                    // Note: Keyberon expects cols as input and rows as output, but we are the opposite so swap them.
                    let events = self.kb.debouncer.events(pressed).map(|e| e.transform(|x, y| (y, x)));

                    // Processes each event, notifying the layout of state change.
                    // If there was any event, we know we have a new report to produce.
                    let mut changed = false;
                    for event in events {
                        self.kb.layout.event(event);
                        self.kb.layout.tick();
                        changed = true;
                    }

                    // We only want to send a report once on press, and once on release.
                    if changed {
                        // Keyberon layout produces boot/usb protocol; convert to our contiguous payload.
                        return ScanOutcome::Report(self.kb.layout.keycodes().collect::<KbHidReport>().into());
                    }
                }
                Err(_) => {
                    error!("Failed to scan keyboard!");
                    return ScanOutcome::Report(KeyboardInputReport::error(ERROR_UNDEFINED));
                }
            }

            // No events; sleep then scan again.
            // Revisit: Instead of periodic polling which could waste power, could wait for interrupt
            // from any row input.
            Timer::after_millis(self.kb.poll_ms).await;
        }
    }
}

impl<
    'hw,
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    const SUBS: usize,
    E: 'hw,
    INPUT: InputPin<Error = E> + 'hw,
    OUTPUT: OutputPin<Error = E> + 'hw,
    DELAY: FnMut() + 'hw,
> odp_service_common::runnable_service::ServiceRunner<'hw>
    for Runner<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, DELAY>
{
    async fn run(mut self) -> Never {
        loop {
            // Wait until the host powers the keyboard on before scanning.
            while !self.shared.is_powered_on() {
                self.shared.power_changed.wait().await;
            }

            match self.scan_cycle().await {
                ScanOutcome::Report(report) => self.publisher.publish(report).await,
                // Powered off mid-cycle; loop back around to re-gate on power.
                ScanOutcome::Unpowered => {}
            }
        }
    }
}

/// GPIO keyboard control handle.
///
/// The matching [`Runner`] must be run for the keyboard to produce input reports.
pub struct Service<
    'hw,
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    const SUBS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    LED: OutputPin,
    DELAY: FnMut(),
> {
    shared: &'hw Shared<SUBS>,
    led_cfg: LedConfig<LED>,
    _phantom: PhantomData<Runner<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, DELAY>>,
}

impl<
    'hw,
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    const SUBS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    LED: OutputPin,
    DELAY: FnMut(),
> Service<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, LED, DELAY>
{
    /// Creates a new GPIO keyboard control handle and its associated scanning runner.
    ///
    /// You must run the returned [`Runner`] (e.g. via `spawn_service!`) for the keyboard to produce
    /// input reports. Pass the returned control handle to a relay such as
    /// [`crate::KeyboardHidRelay`].
    ///
    /// # Errors
    ///
    /// Returns [`KeyboardInitError::TooManyRowsForDeghosting`] if deghosting is enabled for more
    /// than 128 rows, or [`KeyboardInitError::Matrix`] if the key matrix cannot be initialized.
    pub async fn new(
        resources: &'hw mut Resources<SUBS>,
        kb_cfg: KeyboardConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>,
        led_cfg: LedConfig<LED>,
    ) -> Result<(Self, Runner<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, DELAY>), KeyboardInitError<E>> {
        if kb_cfg.deghost && NROWS > 128 {
            return Err(KeyboardInitError::TooManyRowsForDeghosting);
        }

        let kb = KeyberonConfig::try_from(kb_cfg).map_err(KeyboardInitError::Matrix)?;
        let shared: &'hw Shared<SUBS> = resources.inner.insert(Shared::new());

        // Panic safety: we just created the pubsub and haven't handed out any references, so we know there is a free publisher slot available.
        #[allow(clippy::expect_used)]
        let publisher = shared
            .reports
            .publisher()
            .expect("newly-constructed channel is guaranteed to have a publisher slot available");

        Ok((
            Service {
                shared,
                led_cfg,
                _phantom: PhantomData,
            },
            Runner { shared, publisher, kb },
        ))
    }

    fn apply_leds(&mut self, flags: LedFlags) -> Result<(), KeyboardError> {
        set_led(&mut self.led_cfg.num_lock, flags.contains(LedFlags::NumLock))?;
        set_led(&mut self.led_cfg.caps_lock, flags.contains(LedFlags::CapsLock))?;
        set_led(&mut self.led_cfg.scroll_lock, flags.contains(LedFlags::ScrollLock))?;
        Ok(())
    }
}

impl<
    'hw,
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    const SUBS: usize,
    E: 'hw,
    INPUT: InputPin<Error = E> + 'hw,
    OUTPUT: OutputPin<Error = E> + 'hw,
    LED: OutputPin + 'hw,
    DELAY: FnMut() + 'hw,
> odp_service_common::runnable_service::Service<'hw>
    for Service<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, LED, DELAY>
{
    type Runner = Runner<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, DELAY>;
    type Resources = Resources<SUBS>;
}

impl<
    's,
    'hw,
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    const SUBS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    LED: OutputPin,
    DELAY: FnMut(),
> KeyboardService<'s> for Service<'hw, NCOLS, NROWS, NLAYERS, SUBS, E, INPUT, OUTPUT, LED, DELAY>
where
    'hw: 's,
{
    type Error = KeyboardError;

    async fn set_leds(&mut self, flags: LedFlags) -> Result<(), Self::Error> {
        self.apply_leds(flags)
    }

    fn set_power_state(&mut self, state: KeyboardPowerState) {
        self.shared.set_power_state(state);
    }

    fn subscriber(&self) -> Result<DynSubscriber<'s, KeyboardInputReport>, embassy_sync::pubsub::Error> {
        self.shared.reports.dyn_subscriber()
    }
}

#[cfg(test)]
mod tests {
    // Panic safety: tests use panic to communicate failure
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]

    extern crate std;

    use core::convert::Infallible;
    use std::cell::RefCell;
    use std::rc::Rc;

    use embedded_hal::digital::{ErrorType, InputPin, OutputPin};
    use keyberon::key_code::KeyCode;

    use super::*;
    use crate::interface::*;
    use odp_service_common::runnable_service::ServiceRunner;

    static LAYERS: Layers<3, 3, 1> = keyberon::layout::layout! {
        {
            [ A B C ]
            [ D E F ]
            [ G H I ]
        }
    };

    #[derive(Default)]
    struct MatrixState {
        active_col: Option<usize>,
        pressed: [[bool; 3]; 3],
    }

    struct MockKeyboard {
        state: Rc<RefCell<MatrixState>>,
    }

    impl MockKeyboard {
        fn new() -> Self {
            Self {
                state: Rc::new(RefCell::new(MatrixState::default())),
            }
        }

        fn rows(&self) -> [MockInputPin; 3] {
            core::array::from_fn(|row| MockInputPin {
                row,
                state: Rc::clone(&self.state),
            })
        }

        fn cols(&self) -> [MockOutputPin; 3] {
            core::array::from_fn(|col| MockOutputPin {
                col,
                state: Rc::clone(&self.state),
            })
        }

        fn press(&self, key: KeyCode) {
            self.set_pressed(key, true);
        }

        fn release(&self, key: KeyCode) {
            self.set_pressed(key, false);
        }

        fn release_all(&self) {
            self.state.borrow_mut().pressed = [[false; 3]; 3];
        }

        fn set_pressed(&self, key: KeyCode, pressed: bool) {
            let (row, col) = match key {
                KeyCode::A => (0, 0),
                KeyCode::B => (0, 1),
                KeyCode::C => (0, 2),
                KeyCode::D => (1, 0),
                KeyCode::E => (1, 1),
                KeyCode::F => (1, 2),
                KeyCode::G => (2, 0),
                KeyCode::H => (2, 1),
                KeyCode::I => (2, 2),
                _ => panic!("key is not present in the mock 3x3 layout"),
            };

            *self
                .state
                .borrow_mut()
                .pressed
                .get_mut(row)
                .and_then(|matrix_row| matrix_row.get_mut(col))
                .expect("mock key coordinates must be within the 3x3 matrix") = pressed;
        }
    }

    struct MockInputPin {
        row: usize,
        state: Rc<RefCell<MatrixState>>,
    }

    impl ErrorType for MockInputPin {
        type Error = Infallible;
    }

    impl InputPin for MockInputPin {
        fn is_high(&mut self) -> Result<bool, Self::Error> {
            self.is_low().map(|low| !low)
        }

        fn is_low(&mut self) -> Result<bool, Self::Error> {
            let state = self.state.borrow();
            Ok(state.active_col.is_some_and(|col| {
                state
                    .pressed
                    .get(self.row)
                    .and_then(|row| row.get(col))
                    .copied()
                    .unwrap_or(false)
            }))
        }
    }

    struct MockOutputPin {
        col: usize,
        state: Rc<RefCell<MatrixState>>,
    }

    impl ErrorType for MockOutputPin {
        type Error = Infallible;
    }

    impl OutputPin for MockOutputPin {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            self.state.borrow_mut().active_col = Some(self.col);
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            let mut state = self.state.borrow_mut();
            if state.active_col == Some(self.col) {
                state.active_col = None;
            }
            Ok(())
        }
    }

    async fn assert_next_report(sub: &mut DynSubscriber<'_, KeyboardInputReport>, expected: [u8; 8]) {
        let report = embassy_time::with_timeout(embassy_time::Duration::from_millis(100), sub.next_message_pure())
            .await
            .expect("timed out waiting for next report");

        assert_eq!(report.as_bytes(), &expected);
    }

    #[test]
    fn scan_keys_on_3x3() {
        const KEYS: [KeyCode; 9] = [
            KeyCode::A,
            KeyCode::B,
            KeyCode::C,
            KeyCode::D,
            KeyCode::E,
            KeyCode::F,
            KeyCode::G,
            KeyCode::H,
            KeyCode::I,
        ];

        let keyboard = MockKeyboard::new();
        let mut resources = Resources::<1>::default();

        embassy_futures::block_on(async {
            let (mut service, runner) = Service::new(
                &mut resources,
                KeyboardConfig {
                    rows: keyboard.rows(),
                    cols: keyboard.cols(),
                    layers: &LAYERS,
                    poll_ms: 1,
                    nb_bounce: 0,
                    delay: || {},
                    deghost: false,
                },
                LedConfig::<MockOutputPin> {
                    num_lock: None,
                    caps_lock: None,
                    scroll_lock: None,
                },
            )
            .await
            .expect("keyboard service initialization failed");

            embassy_futures::select::select(
                async {
                    let mut sub = service.subscriber().expect("failed to create subscriber");
                    service.set_power_state(KeyboardPowerState::On);

                    for key in KEYS {
                        keyboard.press(key);
                        assert_next_report(&mut sub, [0, 0, key as u8, 0, 0, 0, 0, 0]).await;

                        keyboard.release(key);
                        assert_next_report(&mut sub, KeyboardInputReport::default().0).await;
                    }

                    keyboard.press(KeyCode::A);
                    keyboard.press(KeyCode::E);
                    keyboard.press(KeyCode::I);
                    assert_next_report(
                        &mut sub,
                        [0, 0, KeyCode::A as u8, KeyCode::E as u8, KeyCode::I as u8, 0, 0, 0],
                    )
                    .await;

                    keyboard.release_all();
                    assert_next_report(&mut sub, KeyboardInputReport::default().0).await;
                },
                runner.run(),
            )
            .await;
        });
    }
}
