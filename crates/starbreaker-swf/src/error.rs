//! Error type for SWF parsing and font extraction.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SwfError {
    #[error("failed to parse SWF header: {0}")]
    Header(String),

    #[error("failed to decompress SWF body: {0}")]
    Decompress(String),

    #[error("failed to read SWF tags: {0}")]
    Tags(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
