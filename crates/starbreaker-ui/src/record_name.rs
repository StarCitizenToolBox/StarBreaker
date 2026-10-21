//! Record-name extraction from BuildingBlocks canvas/style references.
//!
//! A small, dependency-free utility shared across the resolver, IR, style, and
//! view-selection layers. It lives in its own module (rather than in `pipeline`)
//! so the lower layers — `bb_resolve`, `bb_scene`, `mfd_view` — do not have to
//! depend up into `pipeline` (which itself depends on them).

/// Extract a DataCore record name from a BuildingBlocks `file://` URL or bare
/// name: strips the `file://` scheme, takes the basename, and drops a trailing
/// `.json` (case-insensitively). Bare names pass through unchanged.
pub fn extract_record_name(file_url_or_name: &str) -> String {
    let without_scheme = file_url_or_name
        .strip_prefix("file://")
        .unwrap_or(file_url_or_name);
    let basename = without_scheme.rsplit('/').next().unwrap_or(without_scheme);
    if basename
        .get(basename.len().saturating_sub(5)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".json"))
    {
        basename[..basename.len() - 5].to_string()
    } else {
        basename.to_string()
    }
}
