//! Helpful utilities for the thermal service
use heapless::Deque;

/// Buffer for storing samples
pub struct SampleBuf<T: Default + Copy + core::fmt::Debug, const N: usize> {
    deque: Deque<T, N>,
}

impl<T: Default + Copy + core::fmt::Debug, const N: usize> SampleBuf<T, N> {
    /// Create a new sample buffer
    pub fn create() -> Self {
        Self { deque: Deque::new() }
    }

    /// Insert a new sample into the buffer and evict the oldest
    pub fn push(&mut self, sample: T) {
        if self.deque.is_full() {
            let _ = self.deque.pop_back();
        }

        // There will always be room in the buffer if we get here
        let _ = self.deque.push_front(sample);
    }

    /// Retrieve the most recent sample from the buffer
    pub fn recent(&self) -> T {
        *self.deque.front().unwrap_or(&T::default())
    }
}

impl<const N: usize> SampleBuf<f32, N> {
    pub fn average(&self) -> f32 {
        self.deque.iter().copied().sum::<f32>() / (self.deque.len() as f32)
    }
}

impl<const N: usize> SampleBuf<u16, N> {
    pub fn average(&self) -> u16 {
        self.deque.iter().copied().sum::<u16>() / (self.deque.len() as u16)
    }
}

/// Convert deciKelvin to degrees Celsius
pub const fn dk_to_c(dk: thermal_service_messages::DeciKelvin) -> f32 {
    (dk.get() as f32 / 10.0) - 273.15
}

/// Convert degrees Celsius to deciKelvin
pub const fn c_to_dk(c: f32) -> thermal_service_messages::DeciKelvin {
    let dk = ((c + 273.15) * 10.0) as u32;
    thermal_service_messages::DeciKelvin::new(dk)
}
