// crates/evolution/src/weight.rs
// Strategy weight management utilities.
use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic f64 wrapper: stores f64 as u64 bits for lock-free access.
/// Used for the learning rate in EvolutionEngine.
pub struct AtomicF64 {
    bits: AtomicU64,
}

impl AtomicF64 {
    pub fn new(value: f64) -> Self {
        Self {
            bits: AtomicU64::new(value.to_bits()),
        }
    }

    pub fn load(&self, ordering: Ordering) -> f64 {
        f64::from_bits(self.bits.load(ordering))
    }

    pub fn store(&self, value: f64, ordering: Ordering) {
        self.bits.store(value.to_bits(), ordering);
    }

    pub fn compare_exchange(
        &self,
        current: f64,
        new: f64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<f64, f64> {
        match self.bits.compare_exchange(
            current.to_bits(),
            new.to_bits(),
            success,
            failure,
        ) {
            Ok(_) => Ok(new),
            Err(bits) => Err(f64::from_bits(bits)),
        }
    }
}

/// Clamp a value to the range [min, max].
#[inline]
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Compute adaptive learning rate: lr_t = lr_0 / sqrt(n + 1)
#[inline]
pub fn adaptive_lr(base_lr: f64, step_count: u64) -> f64 {
    base_lr / ((step_count + 1) as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_f64_roundtrip() {
        let a = AtomicF64::new(0.5);
        assert!((a.load(Ordering::Relaxed) - 0.5).abs() < f64::EPSILON);
        a.store(0.25, Ordering::Relaxed);
        assert!((a.load(Ordering::Relaxed) - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn test_clamp() {
        assert!((clamp(0.5, -1.0, 1.0) - 0.5).abs() < f64::EPSILON);
        assert!((clamp(5.0, -1.0, 1.0) - 1.0).abs() < f64::EPSILON);
        assert!((clamp(-5.0, -1.0, 1.0) - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_adaptive_lr_decay() {
        let lr0 = adaptive_lr(0.1, 0);
        assert!((lr0 - 0.1).abs() < f64::EPSILON);
        let lr99 = adaptive_lr(0.1, 99);
        assert!(lr99 < lr0); // Should decay
        assert!(lr99 > 0.0);
    }
}
