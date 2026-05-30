//! Guardrail test: fail when `src/ui_pipeline.rs` exceeds 400 lines.

use std::fs;
use std::path::PathBuf;

const MAX_LINES: usize = 400;

#[test]
fn rust_source_files_stay_under_line_cap() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let file = manifest_dir.join("src/ui_pipeline.rs");
    let contents =
        fs::read_to_string(&file).unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
    let line_count = contents.lines().count();

    assert!(
        line_count <= MAX_LINES,
        "line-count guard failed: src/ui_pipeline.rs has {line_count} lines (max {MAX_LINES})"
    );
}
