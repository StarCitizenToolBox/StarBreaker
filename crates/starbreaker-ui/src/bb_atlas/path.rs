//! Path normalization and path-segment rewrite helpers for atlas lookups.

/// Canonicalise a BuildingBlocks asset path.
///
/// Lowercases, converts backslashes to forward slashes, strips leading `./`,
/// and collapses a doubled `data/data/` prefix.
pub fn canonicalise_path(raw: &str) -> String {
    let s = raw.trim().replace('\\', "/").to_lowercase();
    let s = s.strip_prefix("./").unwrap_or(&s);
    let s = if s.starts_with("data/data/") {
        s.strip_prefix("data/").unwrap_or(s)
    } else {
        s
    };
    if s.ends_with(".tif") {
        format!("{}.dds", &s[..s.len() - 4])
    } else {
        s.to_string()
    }
}

pub(super) fn extension_of(path: &str) -> &str {
    path.rfind('.').map(|i| &path[i + 1..]).unwrap_or("")
}

pub(super) fn replace_mfd_segment(canonical: &str, replacement: &str) -> String {
    let marker = "/mfd/";
    let Some(mfd_pos) = canonical.find(marker) else {
        return canonical.to_string();
    };
    let after_mfd = &canonical[mfd_pos + marker.len()..];
    if after_mfd.is_empty() {
        return canonical.to_string();
    }
    let next_slash = after_mfd.find('/').unwrap_or(after_mfd.len());
    let rest = &after_mfd[next_slash..];
    format!(
        "{}{}{}{}",
        &canonical[..mfd_pos],
        marker,
        replacement.to_uppercase(),
        rest
    )
}
