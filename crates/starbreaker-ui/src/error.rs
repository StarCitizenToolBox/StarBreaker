//! Error types for `starbreaker-ui`.
//!
//! [`UiError`] covers all failure modes across SWF parsing, image decoding, I/O,
//! and unsupported tag variants.

use thiserror::Error;

/// Unified error type for all `starbreaker-ui` operations.
#[derive(Debug, Error)]
pub enum UiError {
    /// The SWF byte stream could not be parsed (decompression or structural error).
    #[error("SWF parse error: {0}")]
    SwfParse(String),

    /// A SWF tag or feature variant that is not yet supported was encountered.
    #[error("unsupported SWF tag/feature: {0}")]
    UnsupportedTag(String),

    /// An I/O error occurred while reading SWF or image data.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The embedded image data could not be decoded.
    #[error("image decode error: {0}")]
    ImageDecode(#[from] image::ImageError),

    /// A canvas record could not be parsed from its JSON representation.
    #[error("canvas parse error: {0}")]
    ParseError(String),

    /// A cyclic sub-canvas reference was detected during widget tree resolution.
    ///
    /// The inner string is the GUID of the canvas that was revisited.
    #[error("cyclic canvas reference detected for GUID {0}")]
    CycleDetected(String),

    /// The caller-provided canvas-fetch callback returned an error.
    #[error("canvas fetch failed for GUID {guid}: {source}")]
    FetchFailed {
        guid: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The widget tree resolver exceeded its maximum expansion depth.
    #[error("canvas expansion exceeded max depth {max_depth} at GUID {guid}")]
    MaxDepthExceeded { guid: String, max_depth: usize },
}

impl From<swf::error::Error> for UiError {
    fn from(e: swf::error::Error) -> Self {
        Self::SwfParse(e.to_string())
    }
}
