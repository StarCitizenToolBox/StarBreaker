//! Lightweight metadata inspection helpers for GFx/SWF source assets.

use crate::error::GfxResult;
use crate::parser::parse_gfx;
use crate::types::{GfxFile, GfxHeader};

/// Compact metadata dump suitable for CLI tooling and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GfxMetadata {
    /// Parsed container header.
    pub header: GfxHeader,
    /// Number of imported resources discovered by the current parser.
    pub import_count: usize,
    /// Number of symbols discovered by the current parser.
    pub symbol_count: usize,
    /// Number of top-level tags scanned by the current parser.
    pub tag_count: usize,
    /// Number of bytecode tags preserved without execution.
    pub bytecode_tag_count: usize,
}

impl GfxMetadata {
    /// Build metadata from an already parsed file.
    pub fn from_file(file: &GfxFile) -> Self {
        Self {
            header: file.header.clone(),
            import_count: file.imports.len(),
            symbol_count: file.symbols.symbols.len(),
            tag_count: file.tags.len(),
            bytecode_tag_count: file.bytecode.len(),
        }
    }
}

/// Parse a container and return a compact metadata summary without rendering.
pub fn dump_metadata(bytes: &[u8]) -> GfxResult<GfxMetadata> {
    let file = parse_gfx(bytes)?;
    Ok(GfxMetadata::from_file(&file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GfxSignature;

    #[test]
    fn dumps_header_metadata() {
        let mut bytes = b"GFX\x08\0\0\0\0".to_vec();
        bytes.extend_from_slice(&[0x30, 0x00, 0x06, 0x00, 0x00]);
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        let len = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&len.to_le_bytes());
        let metadata = dump_metadata(&bytes).expect("valid header");

        assert_eq!(metadata.header.signature, GfxSignature::Gfx);
        assert_eq!(metadata.header.version, 8);
        assert_eq!(metadata.import_count, 0);
        assert_eq!(metadata.symbol_count, 0);
        assert_eq!(metadata.tag_count, 1);
    }
}
