//! MFD frame-canvas aspect-ratio derivation.
//!
//! Provides [`frame_canvas_aspect`], used by `compile_ir_for_binding` to derive
//! the physical screen aspect (h/w) from the frame canvas record when a binding
//! wraps a content canvas inside a distinct MFD frame canvas.

use super::CanvasFetcher;

/// Aspect (height / width) of the frame canvas referenced by `frame_guid`.
///
/// Returns `None` when there is no distinct frame canvas (absent, or identical
/// to the content canvas) or the frame record has no usable authored size.
/// Callers fall back to SWF/stage-driven sizing in that case.
pub(crate) fn frame_canvas_aspect(
    frame_guid: Option<&str>,
    content_guid: Option<&str>,
    fetcher: &dyn CanvasFetcher,
) -> Option<f32> {
    let frame = frame_guid.filter(|g| !g.is_empty())?;
    // Only a frame that differs from the rendered content canvas defines a
    // separate screen shape; a single-canvas binding has no wrapping frame.
    if content_guid.filter(|g| !g.is_empty()) == Some(frame) {
        return None;
    }
    let json = fetcher.fetch_canvas_json(frame).ok()?;
    let size = json
        .get("_RecordValue_")
        .and_then(|rv| rv.get("size"))
        .or_else(|| json.get("size"))?;
    let w = size.get("x").and_then(|v| v.as_f64())? as f32;
    let h = size.get("y").and_then(|v| v.as_f64())? as f32;
    if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
        Some(h / w)
    } else {
        None
    }
}
