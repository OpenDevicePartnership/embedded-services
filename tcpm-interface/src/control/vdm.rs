//! VDM-related control types

/// Length of the Other VDM data
pub const OTHER_VDM_LEN: usize = 29;
/// Length of the Attention VDM data
pub const ATTN_VDM_LEN: usize = 9;
/// maximum number of data objects in a VDM
pub const MAX_NUM_DATA_OBJECTS: usize = 7; // 7 VDOs of 4 bytes each

/// Other Vdm data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OtherVdm {
    /// Other VDM data
    pub data: [u8; OTHER_VDM_LEN],
}

impl Default for OtherVdm {
    fn default() -> Self {
        Self {
            data: [0; OTHER_VDM_LEN],
        }
    }
}

impl From<OtherVdm> for [u8; OTHER_VDM_LEN] {
    fn from(vdm: OtherVdm) -> Self {
        vdm.data
    }
}

impl From<[u8; OTHER_VDM_LEN]> for OtherVdm {
    fn from(data: [u8; OTHER_VDM_LEN]) -> Self {
        Self { data }
    }
}

/// Attention Vdm data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AttnVdm {
    /// Attention VDM data
    pub data: [u8; ATTN_VDM_LEN],
}

impl Default for AttnVdm {
    fn default() -> Self {
        Self {
            data: [0; ATTN_VDM_LEN],
        }
    }
}

impl From<AttnVdm> for [u8; ATTN_VDM_LEN] {
    fn from(vdm: AttnVdm) -> Self {
        vdm.data
    }
}

impl From<[u8; ATTN_VDM_LEN]> for AttnVdm {
    fn from(data: [u8; ATTN_VDM_LEN]) -> Self {
        Self { data }
    }
}

/// Send VDM data
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SendVdm {
    /// initiating a VDM sequence
    pub initiator: bool,
    /// VDO count
    pub vdo_count: u8,
    /// VDO data
    pub vdo_data: [u32; MAX_NUM_DATA_OBJECTS],
}

impl SendVdm {
    /// Create a new blank VDM
    pub const fn new() -> Self {
        Self {
            initiator: false,
            vdo_count: 0,
            vdo_data: [0; MAX_NUM_DATA_OBJECTS],
        }
    }
}

impl Default for SendVdm {
    fn default() -> Self {
        Self::new()
    }
}
