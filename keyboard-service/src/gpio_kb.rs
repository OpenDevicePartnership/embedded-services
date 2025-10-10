//! A configurable GPIO keyboard which can be used for the keyboard service.
//! If this does not meets the user's needs, the user can implement the `HidKeyboard` trait
//! for their own specific use case.
//!
//! Currently there is no software-implemented deghosting, relying on that to be done
//! in hardware (e.g. diode per switch). Will need to investigate more if there are ways to create
//! a configurable software-implemented deghosting strategy.
use super::HidKeyboard;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_services::GlobalRawMutex;
use embedded_services::hid;
use embedded_services::info;
use keyberon::debounce::Debouncer;
use keyberon::key_code::KbHidReport;
use keyberon::layout::Layout;
pub use keyberon::layout::{Layers, layout};
use keyberon::matrix::Matrix;

// Currently hard cap this to 6 since Keyberon only supports 6 keys
// If move away from Keyberon this can be changed and allow user to configure
const MAX_KEYS: usize = 6;

// I2C input reports begin with 2 byte length of the report
const REPORT_LEN_SZ: usize = 2;

// Indicates the report ID (a single device like a keyboard might have multiple report types)
// If only a single type, can be omitted. But include it anyway for future-proofing.
const REPORT_ID_SZ: usize = 1;

// A single byte represents the state of 8 key modifiers
const REPORT_KEYMOD_SZ: usize = 1;

// The size of a report is the sum of all the above
const REPORT_MAX_SZ: usize = REPORT_LEN_SZ + REPORT_ID_SZ + REPORT_KEYMOD_SZ + MAX_KEYS;

// A input report
const REPORT_ID: u8 = 1;

// This is a basic report descriptor that defines a single keyboard report with 6 keys
// Revisit: Could also allow user to pass in a custom report descriptor
// Will also need to be expanded to support output reports like LEDs
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
    // End Collection
    // Revisit: LED output reports and consumer reports... but can we make that generic?
    0xC0,
];

// A HID input report in the format HID over i2c expects
#[derive(Default)]
struct HidI2cReport([u8; REPORT_MAX_SZ]);

impl HidI2cReport {
    // Conenience for the raw bytes
    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

// HID over i2c expects input reports in different format than keyberon provides so need to convert
impl From<KbHidReport> for HidI2cReport {
    fn from(keyberon: KbHidReport) -> Self {
        // Note: Keyberon uses boot/usb protocol which is [0:modifers, 1:reserved, 2..8: usage codes]
        let keyberon = keyberon.as_bytes();
        let mut buf = Self::default().0;

        // Report length
        buf[0..REPORT_LEN_SZ].copy_from_slice(&(REPORT_MAX_SZ as u16).to_le_bytes());

        // Report type/id
        buf[2] = REPORT_ID;

        // Key modifiers
        buf[3] = keyberon[0];

        // Key usage codes (keyberon only supports 6 keys)
        buf[4..10].copy_from_slice(&keyberon[2..8]);

        Self(buf)
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

/// A HID-aware GPIO keyboard ready to be used by the Keyboard Service.
pub struct GpioKeyboard<
    const NCOLS: usize,
    const NROWS: usize,
    const NLAYERS: usize,
    E,
    INPUT: InputPin<Error = E>,
    OUTPUT: OutputPin<Error = E>,
    DELAY: FnMut(),
> {
    kb_cfg: KeyberonConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>,
    hid_cfg: HidConfig,
    report: HidI2cReport,
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
    DELAY: FnMut(),
> GpioKeyboard<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>
{
    /// Create a new instance of a GPIO Keyboard with given configuration.
    pub fn new(
        kb_cfg: KeyboardConfig<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>,
        hid_cfg: HidConfig,
    ) -> Result<Self, E> {
        Ok(Self {
            kb_cfg: KeyberonConfig::try_from(kb_cfg)?,
            hid_cfg,
            report: HidI2cReport::default(),
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
    DELAY: FnMut(),
> HidKeyboard for GpioKeyboard<NCOLS, NROWS, NLAYERS, E, INPUT, OUTPUT, DELAY>
{
    fn register_file(&self) -> hid::RegisterFile {
        // Don't need anything special do use the default
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
            w_max_input_length: REPORT_MAX_SZ as u16,
            w_output_register: self.register_file().output_reg,
            // We don't currently support output reports with this keyboard
            w_max_output_length: 0,
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

    async fn scan(&mut self) -> &[u8] {
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
                        // Revisit: This collects into a keyberon::KbHidReport which we have to then convert
                        // to HID i2c format. We could manually collect directly into our format, but then
                        // would also need to manually handle modifier keys.
                        let report = self.kb_cfg.layout.keycodes().collect::<KbHidReport>();
                        let report = HidI2cReport::from(report);
                        self.report = report;

                        // We have a new report, so break
                        break;
                    }
                } else {
                    embedded_services::error!("Failed to scan keyboard!");
                }

                // If no events, sleep then scan again
                // Revisit: Instead of periodic polling which could waste power, could wait for interrupt
                // from any row input.
                Timer::after_millis(self.kb_cfg.poll_ms).await;
            }
        };

        // If we hit the idle rate before a new report is generated, provide the latest report
        embassy_futures::select::select(idle, scan).await;

        print_report(&self.report);
        self.report.as_bytes()
    }

    async fn reset(&mut self) {
        self.report_freq = hid::ReportFreq::Infinite;
    }

    async fn set_power_state(&mut self, power_state: hid::PowerState) {
        self.power_state = power_state;

        // Signal to scanner it can start now
        if power_state == hid::PowerState::On {
            self.scan_signal.signal(());
        }
    }

    async fn set_idle(&mut self, _report_id: hid::ReportId, report_freq: hid::ReportFreq) {
        self.report_freq = report_freq;
    }

    fn get_idle(&self, _report_id: hid::ReportId) -> hid::ReportFreq {
        self.report_freq
    }

    async fn set_protocol(&mut self, _protocol: hid::Protocol) {
        // NOP
        // Only support Report protocol
    }

    fn get_protocol(&self) -> hid::Protocol {
        hid::Protocol::Report
    }

    async fn vendor_cmd(&mut self) {
        // NOP
        // No vendor-defined commands for this implementation
    }

    async fn set_report(
        &mut self,
        _report_type: hid::ReportType,
        _report_id: hid::ReportId,
        _buf: &embedded_services::buffer::SharedRef<'static, u8>,
    ) {
        // NOP
        // Do not currently support Output/Feature reports
    }

    async fn get_report(&self, report_type: hid::ReportType, _report_id: hid::ReportId) -> &[u8] {
        match report_type {
            hid::ReportType::Input => self.report.as_bytes(),
            // We don't currently support feature reports
            _ => &[0x00],
        }
    }
}

fn print_report(report: &HidI2cReport) {
    let report = report.as_bytes();
    let modifiers = report[3];
    let keys = &report[4..10];

    if modifiers != 0 {
        info!("Modifiers: 0x{:X}", modifiers);
    }

    for k in keys.iter() {
        if *k != 0 {
            info!("Key pressed: 0x{:X}", k);
        }
    }
}
