//! Error types for GFx/SWF inspection and future rendering stages.

use thiserror::Error;

/// Result alias used by the GFx inspection API.
pub type GfxResult<T> = Result<T, GfxError>;

/// Errors returned while parsing, resolving, or rendering UI source assets.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GfxError {
    /// The input cannot be interpreted as a valid GFx/SWF container.
    #[error("malformed GFx/SWF file: {reason}")]
    MalformedFile { reason: String },

    /// The file contains a valid tag shape that this crate does not decode yet.
    #[error("unsupported GFx/SWF tag {tag}: {reason}")]
    UnsupportedTag { tag: u16, reason: String },

    /// A referenced imported movie/resource is not available to the caller.
    #[error("missing imported asset: {path}")]
    MissingImportedAsset { path: String },

    /// A referenced texture cannot be resolved from the source archive.
    #[error("missing referenced texture: {path}")]
    MissingReferencedTexture { path: String },

    /// The requested output depends on runtime game state or unsupported code.
    #[error("unsupported runtime-only feature: {feature}")]
    UnsupportedRuntimeFeature { feature: String },

    /// PNG encoding failed.
    #[error("image encode failed: {reason}")]
    ImageEncode { reason: String },
}

impl GfxError {
    pub(crate) fn malformed(reason: impl Into<String>) -> Self {
        Self::MalformedFile {
            reason: reason.into(),
        }
    }
}
