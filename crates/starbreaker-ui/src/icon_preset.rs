//! Resolve a `BB_IconWidgetPreset` enum name (e.g. `"ArrowCaratLeft"`) to its
//! vector-icon asset path.
//!
//! A `BuildingBlocks_WidgetIcon` with an empty `svgFill.svgPath` selects its
//! glyph by `iconProperties.iconPreset`, a `BB_IconWidgetPreset` enum value. The
//! engine maps that enum's integer to an SVG via the standard icon widget's
//! `BindingsStringFromIntegerSwitch` table (see
//! `…/modularkit/standard/widgets/iconwidgetstandard.json`, operation index 2).
//!
//! This table is the resolved composition of those two pieces of game data: the
//! `BB_IconWidgetPreset` enum (name → integer, dumped from `Data/Game2.dcb` via
//! `Database::enum_options`) joined with the `IconWidgetStandard` switch table
//! (integer → asset path). Both are stable engine reference data, captured here
//! the same way `bb_brand_apply::colors` captures the `BB_ColorStyle` enum order,
//! so preset icons resolve without a live DataCore handle. `_None` (enum 0) has
//! no asset and is intentionally absent.

/// Shared directory for every standard icon-widget vector asset.
const ICON_WIDGET_DIR: &str = "UI/Textures/Vector/General/ModularKit/Widgets/IconWidget/";

/// Resolve a `BB_IconWidgetPreset` enum name to its full vector-icon asset path,
/// or `None` for `_None`/unknown presets. Matching is case-insensitive.
pub fn svg_path_for_preset(preset_name: &str) -> Option<String> {
    icon_widget_preset_file(preset_name).map(|file| format!("{ICON_WIDGET_DIR}{file}"))
}

/// Map a `BB_IconWidgetPreset` enum name to its asset filename within
/// [`ICON_WIDGET_DIR`].
fn icon_widget_preset_file(preset_name: &str) -> Option<&'static str> {
    // Case-insensitive lookup: scene JSON casing is authoritative, but be lenient.
    PRESET_FILES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(preset_name))
        .map(|(_, file)| *file)
}

/// `(BB_IconWidgetPreset name, asset filename)` pairs, enum-value order.
const PRESET_FILES: &[(&str, &str)] = &[
    ("ArrowHollowUp", "arrow_hollow_up.svg"),
    ("ArrowHollowRight", "arrow_hollow_right.svg"),
    ("ArrowHollowDown", "arrow_hollow_down.svg"),
    ("ArrowHollowLeft", "arrow_hollow_left.svg"),
    ("ArrowHollowCurvedLeft", "arrow_hollow_curved_left.svg"),
    ("ArrowHollowCurvedRight", "arrow_hollow_curved_right.svg"),
    ("ArrowHollowCurvedDoubleLeft", "arrow_hollow_curved_double_left.svg"),
    ("ArrowHollowCurvedDoubleRight", "arrow_hollow_curved_double_right.svg"),
    ("ArrowHollowCurvedDownLeft", "arrow_hollow_curved_downleft.svg"),
    ("ArrowHollowCurvedDownRight", "arrow_hollow_curved_downright.svg"),
    ("ArrowUp", "arrow_up.svg"),
    ("ArrowRight", "arrow_right.svg"),
    ("ArrowDown", "arrow_down.svg"),
    ("ArrowLeft", "arrow_left.svg"),
    ("ArrowUpLeft", "arrow_upleft.svg"),
    ("ArrowUpRight", "arrow_upright.svg"),
    ("ArrowDownRight", "arrow_downright.svg"),
    ("ArrowDownLeft", "arrow_downleft.svg"),
    ("ArrowHookLeft", "arrow_hook_left.svg"),
    ("ArrowHookRight", "arrow_hook_right.svg"),
    ("ArrowDiamond", "arrow_diamond.svg"),
    ("ArrowSquare", "arrow_square.svg"),
    ("ArrowExpandDownUp", "arrow_expand_downup.svg"),
    ("ArrowExpandUpDown", "arrow_expand_updown.svg"),
    ("ArrowCurvedLeft", "arrow_curved_left.svg"),
    ("ArrowCurvedRight", "arrow_curved_right.svg"),
    ("ArrowCurvedDoubleLeft", "arrow_curved_double_left.svg"),
    ("ArrowCurvedDoubleRight", "arrow_curved_double_right.svg"),
    ("ArrowCurvedDownLeft", "arrow_curved_downleft.svg"),
    ("ArrowCurvedDownRight", "arrow_curved_downright.svg"),
    ("ArrowFullCircleCCW", "arrow_fullcircle_ccw.svg"),
    ("ArrowFullCircleCW", "arrow_fullcircle_cw.svg"),
    ("ArrowHalfCircleCCW", "arrow_halfcircle_ccw.svg"),
    ("ArrowHalfCircleCW", "arrow_halfcircle_cw.svg"),
    ("ArrowHalfCircleExclamationCCW", "arrow_halfcircle_exclamation_ccw.svg"),
    ("ArrowHalfCircleExclamationCW", "arrow_halfcircle_exclamation_cw.svg"),
    ("ArrowCaratUp", "arrow_carat_up.svg"),
    ("ArrowCaratRight", "arrow_carat_right.svg"),
    ("ArrowCaratDown", "arrow_carat_down.svg"),
    ("ArrowCaratLeft", "arrow_carat_left.svg"),
    ("ArrowCaratDoubleUp", "arrow_carat_double_up.svg"),
    ("ArrowCaratDoubleRight", "arrow_carat_double_right.svg"),
    ("ArrowCaratDoubleDown", "arrow_carat_double_down.svg"),
    ("ArrowCaratDoubleLeft", "arrow_carat_double_left.svg"),
    ("ArrowEncasedUp", "arrow_encased_up.svg"),
    ("ArrowEncasedRight", "arrow_encased_right.svg"),
    ("ArrowEncasedDown", "arrow_encased_down.svg"),
    ("ArrowEncasedLeft", "arrow_encased_left.svg"),
    ("ArrowEncasedUpLeft", "arrow_encased_upleft.svg"),
    ("ArrowEncasedUpRight", "arrow_encased_upright.svg"),
    ("ArrowEncasedDownRight", "arrow_encased_downright.svg"),
    ("ArrowEncasedDownLeft", "arrow_encased_downleft.svg"),
    ("ArrowEncasedHookLeft", "arrow_encased_hook_left.svg"),
    ("ArrowEncasedHookRight", "arrow_encased_hook_right.svg"),
    ("ArrowEncasedCaratUp", "arrow_encased_carat_up.svg"),
    ("ArrowEncasedCaratRight", "arrow_encased_carat_right.svg"),
    ("ArrowEncasedCaratDown", "arrow_encased_carat_down.svg"),
    ("ArrowEncasedCaratLeft", "arrow_encased_carat_left.svg"),
    ("ArrowEncasedCaratDoubleUp", "arrow_encased_carat_double_up.svg"),
    ("ArrowEncasedCaratDoubleRight", "arrow_encased_carat_double_right.svg"),
    ("ArrowEncasedCaratDoubleDown", "arrow_encased_carat_double_down.svg"),
    ("ArrowEncasedCaratDoubleLeft", "arrow_encased_carat_double_left.svg"),
    ("GeneralCheckmark", "general_checkmark.svg"),
    ("GeneralCircleFilled", "general_circle_filled.svg"),
    ("GeneralX", "general_x.svg"),
    ("GeneralInfo", "general_info.svg"),];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_footer_nav_carats_to_their_svgs() {
        assert_eq!(
            svg_path_for_preset("ArrowCaratLeft").as_deref(),
            Some("UI/Textures/Vector/General/ModularKit/Widgets/IconWidget/arrow_carat_left.svg")
        );
        assert_eq!(
            svg_path_for_preset("ArrowCaratRight").as_deref(),
            Some("UI/Textures/Vector/General/ModularKit/Widgets/IconWidget/arrow_carat_right.svg")
        );
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(
            svg_path_for_preset("generalx").as_deref(),
            Some("UI/Textures/Vector/General/ModularKit/Widgets/IconWidget/general_x.svg")
        );
    }

    #[test]
    fn none_and_unknown_presets_resolve_to_none() {
        assert_eq!(svg_path_for_preset("_None"), None);
        assert_eq!(svg_path_for_preset(""), None);
        assert_eq!(svg_path_for_preset("NotARealPreset"), None);
    }
}
