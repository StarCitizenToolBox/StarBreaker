fn main() {
    // Embed commit SHA if available, otherwise derive from git.
    // Both the app version and the bundled Blender addon's installer-stamped
    // VERSION are built from `CARGO_PKG_VERSION + COMMIT_SHA` (see
    // `build_version()` in `commands.rs`), so every new commit produces a
    // unique addon version and the installer auto-detects updates without a
    // manual VERSION bump in the Python __init__.py.
    let commit_sha = if let Ok(commit_sha) = std::env::var("COMMIT_SHA") {
        // CI environment - use provided SHA
        commit_sha[..7].to_string()
    } else {
        // Local development - get SHA from git and check for dirty state
        let git_sha = std::process::Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Check if working tree is dirty
        let is_dirty = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        if is_dirty && git_sha != "unknown" {
            format!("{}-dirty", git_sha)
        } else {
            git_sha
        }
    };

    println!("cargo:rustc-env=COMMIT_SHA={}", commit_sha);

    // Re-run this build script whenever the git HEAD or refs change.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");

    tauri_build::build();
}
