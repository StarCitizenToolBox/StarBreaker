//! Guardrail test: fail when any Rust source file in src/ exceeds 400 lines.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_LINES: usize = 400;

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn rust_source_files_stay_under_line_cap() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");

    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);
    files.sort();

    let mut violations = Vec::new();
    for file in files {
        let contents = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
        let line_count = contents.lines().count();
        if line_count > MAX_LINES {
            let rel = file
                .strip_prefix(&manifest_dir)
                .unwrap_or(&file)
                .display()
                .to_string();
            violations.push(format!("{rel} has {line_count} lines"));
        }
    }

    assert!(
        violations.is_empty(),
        "line-count guard failed (>{MAX_LINES} lines):\n{}",
        violations.join("\n")
    );
}
