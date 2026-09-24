use embedded_usb_pd::vdm::structured::Svid;
use heapless::Vec;

/// Response from the `Discover SVIDs REQ` message and the PortCommandData::GetDiscoveredSvids command.
// Could be changed to hold the heapless::Vec directly if they were Copy or if PortResponseData was not Copy
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DiscoveredSvids {
    num_sop: usize,
    sop: [Svid; Self::NUM_SVIDS],

    num_sop_prime: usize,
    sop_prime: [Svid; Self::NUM_SVIDS],
}

impl DiscoveredSvids {
    /// The number of SVIDs that can be reported in a single DiscoveredSvids response.
    pub const NUM_SVIDS: usize = 8;

    /// Create a new response object from `sop` and `sop_prime`.
    pub fn new(sop: Vec<Svid, { Self::NUM_SVIDS }>, sop_prime: Vec<Svid, { Self::NUM_SVIDS }>) -> Self {
        let num_sop = sop.len();
        let num_sop_prime = sop_prime.len();

        let mut sop_array = [Svid(0); _];
        for (svid, dest) in sop.into_iter().zip(sop_array.iter_mut()) {
            *dest = svid;
        }

        let mut sop_prime_array = [Svid(0); _];
        for (svid, dest) in sop_prime.into_iter().zip(sop_prime_array.iter_mut()) {
            *dest = svid;
        }

        Self {
            num_sop,
            sop: sop_array,
            num_sop_prime,
            sop_prime: sop_prime_array,
        }
    }

    /// Returns the number of SVIDs discovered on the SOP port partner.
    pub fn number_sop_svids(&self) -> usize {
        self.num_sop
    }

    /// Returns an iterator over the SVIDs discovered on the SOP port partner.
    pub fn svid_sop(&self) -> impl ExactSizeIterator<Item = Svid> {
        self.sop.iter().copied().take(self.num_sop)
    }

    /// Returns the number of SVIDs discovered on the SOP' cable plug.
    pub fn number_sop_prime_svids(&self) -> usize {
        self.num_sop_prime
    }

    /// Returns an iterator over the SVIDs discovered on the SOP' cable plug.
    pub fn svid_sop_prime(&self) -> impl ExactSizeIterator<Item = Svid> {
        self.sop_prime.iter().copied().take(self.num_sop_prime)
    }
}
