//! `BB_ColorStyle` token → palette-slot index, shared by the style cascade
//! (surface fields, `bb_brand_apply::colors`) and the compose renderer
//! (foreground fields, `ir_compose`).
//!
//! The token set IS the `BB_ColorStyle` DataCore enum; its integer value is
//! the direct index into a brand style's `colorStyles` palette array
//! (authoritative dump + re-dump instructions:
//! `crates/starbreaker-ui/docs/ui-architecture-runbook.md` §"BB_ColorStyle colour roles").
//!
//! Audit 2026-06-12: grepping the decompiled record mirror for
//! `"color": "<token>"` finds ONLY enum members — the previously
//! hand-mapped aliases (`Accent3..5`, `Mid`, `Light`, `Highlight`, `Gold`,
//! `Special`, `Surface`, `BG`, `FG`, `Warning`, `Success`, `Negative`,
//! `Text`, `White`, …) occur in no game record and were deleted
//! (2026-06-12 token audit; see crates/starbreaker-ui/docs/ui-process-improvements.md Part D).
//!
//! Role divergences (foreground vs surface) are real engine behaviour but
//! live at the two call sites with their reference citations; this module
//! is the divergence-free enum truth.

/// `BB_ColorStyle` enum index for a token (case-insensitive), or `None`
/// for strings that are not enum members.
pub(crate) fn bb_colour_style_enum_index(token: &str) -> Option<usize> {
    Some(match token.trim().to_ascii_lowercase().as_str() {
        "base" => 0,
        "positive" => 1,
        "moderate" => 2,
        "critical" => 3,
        "accent1" => 4,
        "accent2" => 5,
        "bright" => 6,
        "selected" => 7,
        "disabled" => 8,
        "background" => 9,
        "contactneutral" => 10,
        "contactparty" => 11,
        "contactpositiverep" => 12,
        "contactnegativerep" => 13,
        // The dumped enum spells it "ContactAgressive"; accept the
        // correctly-spelled form too.
        "contactagressive" | "contactaggressive" => 14,
        "contactunknown" => 15,
        "missionobjectives" => 16,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_indices_match_the_authoritative_dump() {
        // Spot-pin the order documented in crates/starbreaker-ui/docs/ui-architecture-runbook.md.
        assert_eq!(bb_colour_style_enum_index("Base"), Some(0));
        assert_eq!(bb_colour_style_enum_index("Moderate"), Some(2));
        assert_eq!(bb_colour_style_enum_index("Accent1"), Some(4));
        assert_eq!(bb_colour_style_enum_index("Accent2"), Some(5));
        assert_eq!(bb_colour_style_enum_index("Bright"), Some(6));
        assert_eq!(bb_colour_style_enum_index("Disabled"), Some(8));
        assert_eq!(bb_colour_style_enum_index("ContactUnknown"), Some(15));
        assert_eq!(bb_colour_style_enum_index("MissionObjectives"), Some(16));
    }

    #[test]
    fn non_enum_tokens_resolve_to_none() {
        for alias in ["Accent5", "Mid", "Light", "Gold", "Surface", "Warning", "White", ""] {
            assert_eq!(bb_colour_style_enum_index(alias), None, "{alias}");
        }
    }
}
