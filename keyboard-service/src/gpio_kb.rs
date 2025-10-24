//! A configurable GPIO keyboard which can be used for the keyboard service.
//! If this does not meets the user's needs, the user can implement the `HidKeyboard` trait
//! for their own specific use case.
use super::HidKeyboard;
use core::borrow::Borrow;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_services::GlobalRawMutex;
use embedded_services::hid;
use embedded_services::{error, warn};
use keyberon::debounce::Debouncer;
use keyberon::key_code::KbHidReport;
use keyberon::layout::Layout;
pub use keyberon::layout::{Layers, layout};
use keyberon::matrix::Matrix;

// Currently hard cap this to 6 since Keyberon only supports 6 keys
// If move away from Keyberon this can be changed and allow user to configure
const KRO: usize = 6;

// A single byte represents the state of 8 key modifiers
const KEYMOD_SZ: usize = 1;

// Don't like how this still needs knowledge of i2c representation
// May need to consider letting hid back end create the hid descriptor
const INPUT_MAX_LEN: usize = super::hid_kb::I2C_REPORT_HEADER_SZ + KEYMOD_SZ + KRO;

// Output reports are the I2C header plus a single byte for LED on/off status
const OUTPUT_MAX_LEN: usize = super::hid_kb::I2C_REPORT_HEADER_SZ + 1;

// An input/output report
const REPORT_ID: u8 = 1;

// This is a basic report descriptor that defines a single keyboard report with 6 keys
// Revisit: Could also allow user to pass in a custom report descriptor
// Revisit: Investigate a struct representation of report descriptors,
// but may prove challenging due to the fact that a strict ordering and length is not defined.
#[rustfmt::skip]
const REPORT_DESCRIPTOR: &[u8] = &[
    // Usage Page (Generic Desktop Ctrls)
    0x05, 0x01,
    // Usage (Keyboard)
    0x09, 0x06,
    // Collection (Application)
    0xA1, 0x01,
    // Report ID (1)
    0x85, REPORT_ID,
    // Usage Page (Keypad)
    0x05, 0x07,
    // Usage Minimum (0xE0)
    0x19, 0xE0,
    // Usage Maximum (0xE7)
    0x29, 0xE7,
    // Logical Minimum (0)
    0x15, 0x00,
    // Logical Maximum (1)
    0x25, 0x01,
    // Report Size (1)
    0x75, 0x01,
    // Report Count (8) (8 modifier keys represented by single bit)
    0x95, 0x08,
    // Input (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x81, 0x02,
    // Usage Minimum (0x00)
    0x19, 0x00,
    // Usage Maximum (0x91)
    0x29, 0x91,
    // Logical Maximum (255)
    0x26, 0xFF, 0x00,
    // Report Size (8)
    0x75, 0x08,
    // Report Count (6) (Keyberon only supports 6 keys)
    0x95, 0x06,
    // Input (Data,Array,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x81, 0x00,
    // LED report
    // Usage Page (LEDs)
    0x05, 0x08,
    // Usage Minimum (Num Lock)
    0x19, 0x01,
    // Usage Maximum (Scroll Lock)
    0x29, 0x03,
    // Report Size (1)
    0x75, 0x01,
    // Report Count (3)
    0x95, 0x03,
    // Logical Maximum (1)
    0x25, 0x01,
    // Output (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x91, 0x02,
    // Report Count (5)
    0x95, 0x05,
    // Output (Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x91, 0x01,
    // End LED report
    // Revisit: Consumer reports... but can we make that generic?
    // End Collection
    0xC0,
];

// Matches the format described by report descriptor
// As in, each LED on/off status represented by single-bit
bitflags::bitflags! {
    pub struct LedFlags: u8 {
        const NumLock = 1 << 0;
        const CapsLock = 1 << 1;
        const ScrollLock = 1 << 2;
        // The host may set any bits
        const _ = !0;
    }
}

fn set_led(led: &mut Option<impl OutputPin>, cond: bool) -> Result<(), super::KeyboardError> {
    if let Some(led) = led {
        if cond {
            led.set_high().map_err(|_| super::KeyboardError::Scan)?;
        } else {
            led.set_low().map_err(|_| super::KeyboardError::Scan)?;
        }
    }

    Ok(())
}

// Note: This is not defined at top-level because operations on const generics is not yet stable
// E.g. `struct HidReport<const KRO: usize>([u8; KRO + 1])` is not currently possible
#[derive(Default)]
struct HidReport([u8; KRO + KEYMOD_SZ]);

impl HidReport {
    fn as_slice(&self) -> super::HidReportSlice<'_> {
        super::HidReportSlice(&self.0)
    }
}

impl From<KbHidReport> for HidReport {
    fn from(keyberon: KbHidReport) -> Self {
        // Note: Keyberon uses boot/usb protocol which is [0:modifers, 1:reserved, 2..8: usage codes]
        let keyberon = keyberon.as_bytes();

        let mut buf = [0; KRO + KEYMOD_SZ];
        buf[0] = keyberon[0];
        buf[1..1 + KRO].copy_from_slice(&keyberon[2..2 + KRO]);

        HidReport(buf)
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

/// Keyboard HID configuration.
pub struct HidConfig {
    /// Vendor ID
    pub vid: u16,
    /// Product ID
    pub pid: u16,
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
    for (c, row) in pressed.iter().enumerate() {
        for (r, &key) in row.iter().enumerate() {
            if key {
                count += 1;
                pressed_bits[c] |= 1 << r;
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

/// A HID-aware GPIO keyboard ready to be used by the Keyboard Service.
pub struct GpioKeyboard<
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    LED: OutputPin,
    DELAY: FnMut(),
> {
    kb_cfg: KeyberonConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>,
    hid_cfg: HidConfig,
    led_cfg: LedConfig<LED>,
    report: HidReport,
    power_state: hid::PowerState,
    scan_signal: Signal<GlobalRawMutex, ()>,
    report_freq: hid::ReportFreq,
}

impl<
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    LED: OutputPin,
    DELAY: FnMut(),
> GpioKeyboard<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, LED, DELAY>
{
    /// Create a new instance of a GPIO Keyboard with given configuration.
    ///
    /// # Panics
    ///
    /// If `deghosting` is enabled in `kb_cfg`, panics if `NROWS > 128`.
    pub fn new(
        kb_cfg: KeyboardConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>,
        hid_cfg: HidConfig,
        led_cfg: LedConfig<LED>,
    ) -> Result<Self, E> {
        // We can only support upto 128 rows for deghosting
        if kb_cfg.deghost {
            assert!(NROWS <= 128);
        }

        Ok(Self {
            kb_cfg: KeyberonConfig::try_from(kb_cfg)?,
            hid_cfg,
            led_cfg,
            report: HidReport::default(),
            power_state: hid::PowerState::Sleep,
            scan_signal: Signal::new(),
            report_freq: hid::ReportFreq::Infinite,
        })
    }
}

impl<
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    LED: OutputPin,
    DELAY: FnMut(),
> HidKeyboard for GpioKeyboard<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, LED, DELAY>
{
    fn register_file(&self) -> hid::RegisterFile {
        // Don't need anything special so use the default
        hid::RegisterFile::default()
    }

    fn hid_descriptor(&self) -> hid::Descriptor {
        const VERSION: u16 = 0x0100;

        hid::Descriptor {
            w_hid_desc_length: hid::DESCRIPTOR_LEN as u16,
            bcd_version: VERSION,
            w_report_desc_length: REPORT_DESCRIPTOR.len() as u16,
            w_report_desc_register: self.register_file().report_desc_reg,
            w_input_register: self.register_file().input_reg,
            w_max_input_length: INPUT_MAX_LEN as u16,
            w_output_register: self.register_file().output_reg,
            w_max_output_length: OUTPUT_MAX_LEN as u16,
            w_command_register: self.register_file().command_reg,
            w_data_register: self.register_file().data_reg,
            w_vendor_id: self.hid_cfg.vid,
            w_product_id: self.hid_cfg.pid,
            w_version_id: VERSION,
        }
    }

    fn report_descriptor(&self) -> &'static [u8] {
        REPORT_DESCRIPTOR
    }

    async fn scan(&mut self) -> Result<super::HidReportSlice<'_>, super::KeyboardError> {
        // Wait until we are told to power on before scanning
        if self.power_state == hid::PowerState::Sleep {
            self.scan_signal.wait().await;
        }

        // Determine the idle rate
        let idle = if let hid::ReportFreq::Msecs(ms) = self.report_freq {
            Timer::after_millis(ms as u64)
        } else {
            // If set to 'infinite', set a timer very far in the future (effectively infinite)
            Timer::after_secs(1_000_000)
        };

        // Polling scan loop
        let scan = async {
            loop {
                // Scan for keys currently pressed
                if let Ok(pressed) = self.kb_cfg.matrix.get_with_delay(&mut self.kb_cfg.delay) {
                    // If ghosting detected, break and report error
                    if self.kb_cfg.deghost && has_ghost(&pressed) {
                        warn!("Key ghosting detected");
                        break Err(super::KeyboardError::Ghosting);
                    }

                    // Run the scan through the debouncer, applying a coordinate transform if provided
                    // Note: Keyberon expects cols as input and rows as output, but we are the opposite so swap them for proper coordinate
                    let events = self
                        .kb_cfg
                        .debouncer
                        .events(pressed)
                        .map(|e| e.transform(|x, y| (y, x)));

                    // Processes each event, notifiying the layout of state change
                    // If there was any event, we know we have a new report to produce
                    let mut changed = false;
                    for event in events {
                        self.kb_cfg.layout.event(event);
                        self.kb_cfg.layout.tick();
                        changed = true;
                    }

                    // We only want to send a report once on press, and once on release
                    // No need to continuously send reports while the key is held down
                    if changed {
                        // Keyberon layout will convert event coordinates to HID usage codes
                        // But keyberon's format follows boot/usb protocol, so we convert it
                        // to a contiguous modifer byte + usage codes array
                        self.report = self.kb_cfg.layout.keycodes().collect::<KbHidReport>().into();
                        break Ok(());
                    }
                } else {
                    error!("Failed to scan keyboard!");
                    break Err(super::KeyboardError::Scan);
                }

                // If no events, sleep then scan again
                // Revisit: Instead of periodic polling which could waste power, could wait for interrupt
                // from any row input.
                Timer::after_millis(self.kb_cfg.poll_ms).await;
            }
        };

        match embassy_futures::select::select(idle, scan).await {
            // Hit the idle limit? Return the most recent report
            embassy_futures::select::Either::First(_) => Ok(self.report.as_slice()),

            // Have a fresh report? Return it
            // Note: We don't return report slice in the loop above as this causes lifetime issues
            embassy_futures::select::Either::Second(Ok(())) => Ok(self.report.as_slice()),

            // Error? Let the HID backend convert it to report for us
            embassy_futures::select::Either::Second(Err(e)) => Err(e),
        }
    }

    async fn reset(&mut self) -> Result<(), super::KeyboardError> {
        self.report_freq = hid::ReportFreq::Infinite;
        Ok(())
    }

    async fn set_power_state(&mut self, power_state: hid::PowerState) -> Result<(), super::KeyboardError> {
        self.power_state = power_state;

        // Signal to scanner it can start now
        if power_state == hid::PowerState::On {
            self.scan_signal.signal(());
        }

        Ok(())
    }

    async fn set_idle(
        &mut self,
        _report_id: hid::ReportId,
        report_freq: hid::ReportFreq,
    ) -> Result<(), super::KeyboardError> {
        self.report_freq = report_freq;
        Ok(())
    }

    fn get_idle(&self, _report_id: hid::ReportId) -> hid::ReportFreq {
        self.report_freq
    }

    async fn set_protocol(&mut self, _protocol: hid::Protocol) -> Result<(), super::KeyboardError> {
        // NOP
        // Only support Report protocol
        Ok(())
    }

    fn get_protocol(&self) -> hid::Protocol {
        hid::Protocol::Report
    }

    async fn vendor_cmd(&mut self) -> Result<(), super::KeyboardError> {
        // NOP
        // No vendor-defined commands for this implementation
        Ok(())
    }

    async fn set_report(
        &mut self,
        report_type: hid::ReportType,
        report_id: hid::ReportId,
        buf: &embedded_services::buffer::SharedRef<'static, u8>,
    ) -> Result<(), super::KeyboardError> {
        match report_type {
            // Received a set output report for LEDs
            hid::ReportType::Output if report_id.0 == REPORT_ID => {
                let buf = buf.borrow();
                let leds: &[u8] = buf.borrow();
                let flags = LedFlags::from_bits_retain(leds[0]);

                set_led(&mut self.led_cfg.num_lock, flags.contains(LedFlags::NumLock))?;
                set_led(&mut self.led_cfg.caps_lock, flags.contains(LedFlags::CapsLock))?;
                set_led(&mut self.led_cfg.scroll_lock, flags.contains(LedFlags::ScrollLock))?;
            }
            // Not currently supported so treat as NOP
            hid::ReportType::Feature => (),
            // Should never receive a set input report command
            hid::ReportType::Input => Err(super::KeyboardError::Command)?,
            // Received set output for unknown report ID, also treat as NOP
            _ => (),
        }

        Ok(())
    }

    fn get_report(&self, report_type: hid::ReportType, _report_id: hid::ReportId) -> super::HidReportSlice<'_> {
        match report_type {
            hid::ReportType::Input => self.report.as_slice(),
            // We don't currently support feature reports
            _ => super::HidReportSlice(&[0x00]),
        }
    }
}
