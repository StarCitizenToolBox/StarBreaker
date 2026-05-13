//! Shared progress reporting for long-running operations.

use std::sync::atomic::{AtomicU32, Ordering};
use parking_lot::Mutex;

/// Thread-safe progress state. Pass `Option<&Progress>` to heavy functions.
/// `None` means no reporting (zero cost). The UI creates one and polls it.
pub struct Progress {
    /// 0–10000 for 0.00%–100.00%
    value: AtomicU32,
    stage: Mutex<String>,
    /// If set, this progress writes scaled values into a parent.
    parent: Option<(*const Progress, f32, f32)>,  // (parent_ptr, base, range)
}

// SAFETY: parent is only read, never written after construction, and points to
// data that outlives this Progress (enforced by SubProgress lifetime).
unsafe impl Send for Progress {}
unsafe impl Sync for Progress {}

impl Progress {
    pub fn new() -> Self {
        Self {
            value: AtomicU32::new(0),
            stage: Mutex::new(String::new()),
            parent: None,
        }
    }

    /// Update progress. `fraction` is 0.0–1.0.
    pub fn report(&self, fraction: f32, message: &str) {
        let clamped = fraction.clamp(0.0, 1.0);

        if let Some((parent_ptr, base, range)) = self.parent {
            let scaled = base + clamped * range;
            // SAFETY: parent outlives self (enforced by SubProgress).
            let parent = unsafe { &*parent_ptr };
            parent.report(scaled, message);
        } else {
            let units = (clamped * 10000.0) as u32;
            let previous = self.value.fetch_max(units, Ordering::Relaxed);
            if units >= previous {
                *self.stage.lock() = message.to_string();
            }
        }
    }

    /// Read current progress.
    pub fn get(&self) -> (f32, String) {
        let v = self.value.load(Ordering::Relaxed) as f32 / 10000.0;
        let s = self.stage.lock().clone();
        (v, s)
    }

    pub fn is_done(&self) -> bool {
        self.value.load(Ordering::Relaxed) >= 10000
    }

    /// Create a sub-progress that maps 0.0–1.0 to `from..to` on this progress.
    /// The returned `Progress` can be passed anywhere `&Progress` is accepted.
    pub fn sub(&self, from: f32, to: f32) -> Progress {
        Progress {
            value: AtomicU32::new(0),
            stage: Mutex::new(String::new()),
            parent: Some((self as *const Progress, from, to - from)),
        }
    }
}

impl Default for Progress {
    fn default() -> Self { Self::new() }
}

/// Helper: report on an `Option<&Progress>`, noop if None.
pub fn report(progress: Option<&Progress>, fraction: f32, message: &str) {
    if let Some(p) = progress {
        p.report(fraction, message);
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    #[test]
    fn progress_reports_are_monotonic() {
        let progress = Progress::new();
        progress.report(0.8, "later phase");
        progress.report(0.4, "older phase");

        let (fraction, stage) = progress.get();
        assert_eq!(fraction, 0.8);
        assert_eq!(stage, "later phase");
    }

    #[test]
    fn subprogress_reports_are_monotonic_on_parent() {
        let progress = Progress::new();
        progress.report(0.6, "parent phase");
        let sub = progress.sub(0.1, 0.2);
        sub.report(0.5, "sub phase");

        let (fraction, stage) = progress.get();
        assert_eq!(fraction, 0.6);
        assert_eq!(stage, "parent phase");
    }

    #[test]
    fn subprogress_maps_fraction_between_endpoints() {
        let progress = Progress::new();
        let sub = progress.sub(0.2, 0.6);

        sub.report(0.5, "sub phase");

        let (fraction, stage) = progress.get();
        assert!((fraction - 0.4).abs() < 0.0001);
        assert_eq!(stage, "sub phase");
    }

    #[test]
    fn nested_subprogress_reports_reach_root_progress() {
        let progress = Progress::new();
        let assembly = progress.sub(0.06, 0.90);
        let decomposed = assembly.sub(0.0, 0.90);

        decomposed.report(0.5, "Writing child assets");

        let (fraction, stage) = progress.get();
        assert!((fraction - 0.438).abs() < 0.0001);
        assert_eq!(stage, "Writing child assets");
    }
}
