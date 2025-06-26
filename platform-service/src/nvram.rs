use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::GlobalRawMutex;

use crate::nvram_valid_range;

// Describes a u32 section of non-volatile RAM
struct Info {
    // the starting address/offset and length in u32 words of the section
    offset: usize,

    guard: Mutex<GlobalRawMutex, ()>,
}

/// Section descriptor table for linking indices to offsets
pub struct Table<const N: usize> {
    sections: [Info; N],
}

impl<const N: usize> Table<N> {
    /// Generate a Section descriptor table of length N
    pub const fn new(valid_offsets: &[usize; N]) -> Self {
        let mut as_info: [Info; N] = [const { Info::new(0) }; N];

        // have to while loop here for const fn
        let mut i = 0;
        while i < N {
            as_info[i].offset = valid_offsets[i];

            i += 1;
        }

        Self { sections: as_info }
    }

    /// Get the index associated with given offset, for passing context key around as needed
    pub fn get_index(&self, offset: usize) -> Option<usize> {
        self.sections
            .iter()
            .enumerate()
            .find_map(|(index, info)| if info.offset == offset { Some(index) } else { None })
    }
}

/// Table initialization errors
#[derive(Copy, Clone, Debug)]
pub enum TableError {
    /// Offset constructed is not accessible per NVRAM implementation
    InvalidOffset(usize),
}

impl Info {
    const fn new(offset: usize) -> Self {
        Self {
            offset,
            guard: Mutex::new(()),
        }
    }
}

/// Guarded handle to section
pub struct ManagedSection {
    info: &'static Info,
}

impl ManagedSection {
    const fn new(info: &'static Info) -> Self {
        Self { info }
    }

    /// Attempt to read an offset from a section. Returns None if the offset is invalid or the section is marked inaccessible
    pub fn read(&self) -> u32 {
        self.info.guard.lock(|_| crate::nvram_read(self.info.offset))
    }

    /// Attempt to write value to offset within section. Returns read(offset) on success, None on failure
    pub fn write(&mut self, value: u32) {
        // mutex guard ensures only one writer to this region at a time
        self.info.guard.lock(|_| crate::nvram_write(self.info.offset, value));
    }
}

static LAYOUT: OnceLock<&'static [Info]> = OnceLock::new();

/// Initialize the NVRAM service with a given table of named sections
pub async fn init<const N: usize>(table: &'static Table<N>) -> Result<(), TableError> {
    for entry in &table.sections {
        if !nvram_valid_range().contains(&entry.offset) {
            return Err(TableError::InvalidOffset(entry.offset));
        }
    }

    LAYOUT.get_or_init(|| &table.sections);
    Ok(())
}

/// General API for attempting to interact with named NVRAM sections
pub async fn lookup_section(index: usize) -> Option<ManagedSection> {
    let layout = LAYOUT.get().await;

    if layout.len() <= index {
        None
    } else {
        Some(ManagedSection::new(&layout[index]))
    }
}
