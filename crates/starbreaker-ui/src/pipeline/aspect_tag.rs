//! Data-driven MFD aspect-ratio responsive layout resolution.
//!
//! Star Engine adapts an MFD's BuildingBlocks layout to the physical screen
//! aspect via two DataCore records (no hardcoded ratios or tag GUIDs here — the
//! values are read from the records at run time):
//!
//! 1. A `BuildingBlocks_AspectRatioLibrary` (e.g. `AspectRatioToTag_MFD`) maps a
//!    continuous aspect ratio (width / height) to the NEAREST authored option's
//!    layout tag (16:9 / 4:3 / 1:1 …). The engine applies that tag to the MFD
//!    root at runtime; downstream responsive style entries gate on it.
//! 2. The MFD content frame (`m_eng_mfdcontent`) carries "Content Canvas
//!    Scaling" `embeddedStyles` whose condition requires an ancestor with the
//!    aspect tag and whose modifiers set the content canvas width to
//!    `SizeX × height` (`WidthBehavior = PercentOfY`). 4:3 → 1.45, 16:9 → 2.0,
//!    1:1 → 1.45 (plus a 0.79 scale).
//!
//! [`nearest_aspect_tag`] implements (1); [`content_scaling_width`] reads (2) for
//! the matched tag. Both are pure over the parsed records so they unit-test
//! without a fetcher (crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md).

/// Width sizing a "Content Canvas Scaling" entry imposes on the content canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ContentScalingWidth {
    /// The `SizeX` multiplier (e.g. 1.45 for 4:3).
    pub size_x: f32,
    /// The authored `WidthBehavior` (expected `"PercentOfY"`: width = size_x × own height).
    pub behavior_is_percent_of_y: bool,
}

/// Return the layout-tag `_RecordId_` of the aspect-ratio option NEAREST to
/// `aspect_w_over_h` in a `BuildingBlocks_AspectRatioLibrary` record.
///
/// `library` is the full record JSON (`_RecordValue_.aspectRatioOptions[]` of
/// `{aspectRatio, tag}`). Returns `None` when the record has no usable options.
pub(super) fn nearest_aspect_tag(library: &serde_json::Value, aspect_w_over_h: f32) -> Option<String> {
    if !aspect_w_over_h.is_finite() || aspect_w_over_h <= 0.0 {
        return None;
    }
    let options = library
        .get("_RecordValue_")
        .unwrap_or(library)
        .get("aspectRatioOptions")
        .and_then(|v| v.as_array())?;

    let mut best: Option<(f32, String)> = None;
    for option in options {
        let ratio = option.get("aspectRatio").and_then(|v| v.as_f64())? as f32;
        let tag_id = option
            .get("tag")
            .and_then(|t| t.get("_RecordId_"))
            .and_then(|v| v.as_str())?;
        let delta = (ratio - aspect_w_over_h).abs();
        if best.as_ref().is_none_or(|(best_delta, _)| delta < *best_delta) {
            best = Some((delta, tag_id.to_string()));
        }
    }
    best.map(|(_, tag_id)| tag_id)
}

/// Read the "Content Canvas Scaling" width sizing a content-frame record imposes
/// when an ancestor carries `tag_id`.
///
/// Scans `frame_record._RecordValue_.embeddedStyles[]` for an entry that (a) is
/// conditioned on an `Ancestor(Tag(tag_id))` selector and (b) carries a `SizeX`
/// `FieldModifierNumber`. Returns its `SizeX` and whether the entry also sets
/// `WidthBehavior = PercentOfY`. `None` when no such entry exists.
pub(super) fn content_scaling_width(
    frame_record: &serde_json::Value,
    tag_id: &str,
) -> Option<ContentScalingWidth> {
    let entries = frame_record
        .get("_RecordValue_")
        .unwrap_or(frame_record)
        .get("embeddedStyles")
        .and_then(|v| v.as_array())?;

    for entry in entries {
        if !entry_conditions_reference_ancestor_tag(entry, tag_id) {
            continue;
        }
        let modifiers = entry.get("modifiers").and_then(|v| v.as_array());
        let Some(modifiers) = modifiers else { continue };
        let size_x = modifiers.iter().find_map(|m| size_x_modifier_value(m));
        let Some(size_x) = size_x else { continue };
        let behavior_is_percent_of_y = modifiers.iter().any(width_behavior_is_percent_of_y);
        return Some(ContentScalingWidth {
            size_x,
            behavior_is_percent_of_y,
        });
    }
    None
}

/// True when any condition in `entry.conditionsList` is (or nests) an
/// `Ancestor` selector containing a `Tag` condition referencing `tag_id`.
fn entry_conditions_reference_ancestor_tag(entry: &serde_json::Value, tag_id: &str) -> bool {
    entry
        .get("conditionsList")
        .and_then(|v| v.as_array())
        .is_some_and(|lists| {
            lists.iter().any(|list| {
                list.get("conditions")
                    .and_then(|v| v.as_array())
                    .is_some_and(|conds| {
                        conds.iter().any(|c| condition_has_ancestor_tag(c, tag_id))
                    })
            })
        })
}

/// Recursively test whether `condition` is an `Ancestor` selector whose nested
/// conditions reference `tag_id` via a `Tag` selector (or contains one through
/// `AllOf`/`AnyOf` nesting).
fn condition_has_ancestor_tag(condition: &serde_json::Value, tag_id: &str) -> bool {
    let cond_type = condition.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
    if cond_type.ends_with("ConditionAncestor") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .is_some_and(|conds| conds.iter().any(|c| condition_references_tag(c, tag_id)));
    }
    // Descend through boolean combinators (AllOf / AnyOf wrap the ancestor test).
    if cond_type.ends_with("Condition")
        && let Some(children) = condition.get("conditions").and_then(|v| v.as_array())
    {
        return children
            .iter()
            .any(|c| condition_has_ancestor_tag(c, tag_id));
    }
    false
}

/// Test whether a condition is a `Tag` selector referencing `tag_id`.
fn condition_references_tag(condition: &serde_json::Value, tag_id: &str) -> bool {
    condition
        .get("tag")
        .and_then(|t| t.get("_RecordId_"))
        .and_then(|v| v.as_str())
        == Some(tag_id)
}

/// `SizeX` value of a `FieldModifierNumber` modifier, or `None`.
fn size_x_modifier_value(modifier: &serde_json::Value) -> Option<f32> {
    let is_number = modifier.get("_Type_").and_then(|v| v.as_str())
        == Some("BuildingBlocks_FieldModifierNumber");
    let field_is_size_x = modifier.get("field").and_then(|v| v.as_str()) == Some("SizeX");
    if is_number && field_is_size_x {
        return modifier.get("value").and_then(|v| v.as_f64()).map(|v| v as f32);
    }
    None
}

/// True when a modifier sets `WidthBehavior` to `PercentOfY`.
fn width_behavior_is_percent_of_y(modifier: &serde_json::Value) -> bool {
    let field = modifier.get("field");
    let is_width_behavior = field
        .and_then(|f| f.get("_Type_"))
        .and_then(|v| v.as_str())
        == Some("BuildingBlocks_FieldModifierEnumeratedTypeWidthBehavior")
        || field.and_then(|f| f.get("value")).and_then(|v| v.as_str()) == Some("PercentOfY");
    is_width_behavior
        && field.and_then(|f| f.get("value")).and_then(|v| v.as_str()) == Some("PercentOfY")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mfd_library() -> serde_json::Value {
        // Synthetic stand-in for AspectRatioToTag_MFD: three options with
        // DISTINCT synthetic tag ids (not real game GUIDs).
        json!({
            "_RecordValue_": {
                "aspectRatioOptions": [
                    {"aspectRatio": 1.777778, "tag": {"_RecordId_": "tag-16x9"}},
                    {"aspectRatio": 1.333333, "tag": {"_RecordId_": "tag-4x3"}},
                    {"aspectRatio": 1.0, "tag": {"_RecordId_": "tag-1x1"}}
                ]
            }
        })
    }

    #[test]
    fn nearest_tag_exact_and_between() {
        let lib = mfd_library();
        assert_eq!(nearest_aspect_tag(&lib, 1.3333).as_deref(), Some("tag-4x3"));
        assert_eq!(nearest_aspect_tag(&lib, 1.0).as_deref(), Some("tag-1x1"));
        assert_eq!(nearest_aspect_tag(&lib, 1.78).as_deref(), Some("tag-16x9"));
        // 1.2 is closer to 1.333 than to 1.0.
        assert_eq!(nearest_aspect_tag(&lib, 1.2).as_deref(), Some("tag-4x3"));
        // 1.15 is closer to 1.0.
        assert_eq!(nearest_aspect_tag(&lib, 1.15).as_deref(), Some("tag-1x1"));
    }

    #[test]
    fn nearest_tag_rejects_bad_input_and_empty() {
        let lib = mfd_library();
        assert_eq!(nearest_aspect_tag(&lib, 0.0), None);
        assert_eq!(nearest_aspect_tag(&lib, f32::NAN), None);
        assert_eq!(nearest_aspect_tag(&json!({"_RecordValue_": {}}), 1.333), None);
    }

    fn content_frame() -> serde_json::Value {
        // Mirrors m_eng_mfdcontent embeddedStyles[9] (4:3) shape.
        json!({
            "_RecordValue_": {
                "embeddedStyles": [
                    {
                        "name": "Content Canvas Scaling (4:3)",
                        "conditionsList": [{
                            "conditions": [{
                                "_Type_": "BuildingBlocks_StyleSelectorConditionAllOfCondition",
                                "conditions": [
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"},
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionAncestor",
                                     "conditions": [
                                        {"_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                         "tag": {"_RecordId_": "tag-4x3"}}
                                     ]},
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                     "tag": {"_RecordId_": "content-canvas"}}
                                ]
                            }]
                        }],
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeX", "value": 1.45},
                            {"field": {"_Type_": "BuildingBlocks_FieldModifierEnumeratedTypeWidthBehavior",
                                       "value": "PercentOfY"}}
                        ]
                    },
                    {
                        "name": "Content Canvas Scaling (16:9)",
                        "conditionsList": [{
                            "conditions": [{
                                "_Type_": "BuildingBlocks_StyleSelectorConditionAncestor",
                                "conditions": [
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                     "tag": {"_RecordId_": "tag-16x9"}}
                                ]
                            }]
                        }],
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeX", "value": 2.0},
                            {"field": {"_Type_": "BuildingBlocks_FieldModifierEnumeratedTypeWidthBehavior",
                                       "value": "PercentOfY"}}
                        ]
                    }
                ]
            }
        })
    }

    #[test]
    fn content_scaling_reads_matching_tag() {
        let frame = content_frame();
        let w = content_scaling_width(&frame, "tag-4x3").expect("4:3 entry");
        assert_eq!(w.size_x, 1.45);
        assert!(w.behavior_is_percent_of_y);

        let w16 = content_scaling_width(&frame, "tag-16x9").expect("16:9 entry");
        assert_eq!(w16.size_x, 2.0);
    }

    #[test]
    fn content_scaling_none_for_unknown_tag() {
        let frame = content_frame();
        assert_eq!(content_scaling_width(&frame, "tag-1x1"), None);
        assert_eq!(content_scaling_width(&json!({"_RecordValue_": {}}), "tag-4x3"), None);
    }
}
