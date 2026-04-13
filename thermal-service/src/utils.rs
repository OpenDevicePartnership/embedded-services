//! Helpful utilities for the thermal service.
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
    /// Returns the average of the samples in the buffer, or 0.0 if the buffer is empty.
    pub fn average(&self) -> f32 {
        let len = self.deque.len();
        if len == 0 {
            return 0.0;
        }
        self.deque.iter().copied().sum::<f32>() / len as f32
    }
}

impl<const N: usize> SampleBuf<u16, N> {
    /// Returns the average of the samples in the buffer, or 0 if the buffer is empty.
    pub fn average(&self) -> u16 {
        let sum: u32 = self.deque.iter().copied().map(u32::from).sum();
        sum.checked_div(self.deque.len() as u32).unwrap_or(0) as u16
    }
}
