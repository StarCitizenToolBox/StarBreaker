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
}

impl From<swf::error::Error> for UiError {
    fn from(e: swf::error::Error) -> Self {
        Self::SwfParse(e.to_string())
    }
}
