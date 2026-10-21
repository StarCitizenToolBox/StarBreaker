//! Gated per-stage timing helper for `[timing][ui]` log lines.
//!
//! Enable with `SB_UI_TIMING=1`. When the variable is absent every call is a
//! zero-overhead pass-through (no allocation, no syscall in optimised builds).

/// Run `f`, emitting `[timing][ui] {label}={elapsed:.3}s` when `SB_UI_TIMING` is set.
pub(crate) fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    if std::env::var("SB_UI_TIMING").is_err() {
        return f();
    }
    let t = std::time::Instant::now();
    let r = f();
    log::info!("[timing][ui] {}={:.3}s", label, t.elapsed().as_secs_f32());
    r
}
