//! Node `inlineStyles` cascade tests: per-node authored style entries apply
//! as the FINAL stage (highest specificity — above brand/default entries),
//! and an inline `FontSize` is recorded so font resolution prefers it over
//! the brand-table standard.

use super::tests_support::make_test_scene;
use super::*;
use serde_json::json;

/// The power screen's `text_BatteryTitle` / `text_OutputTitle` author an
/// unconditional inline `FontSize 30` that must land on the node.
#[test]
fn inline_styles_apply_to_owning_node() {
    let mut scene = make_test_scene();
    scene.nodes.get_mut(&1).unwrap().raw = json!({
        "inlineStyles": [{
            "_Type_": "BuildingBlocks_StyleEntry",
            "name": "New Style",
            "conditionsList": [],
            "modifiers": [{
                "_Type_": "BuildingBlocks_FieldModifierNumber",
                "field": "FontSize",
                "value": 30.0
            }]
        }]
    });
    let brand = BrandStyle {
        identifier: "test_brand".to_string(),
        entries: &[],
        raw: &json!({}),
    };

    apply_brand_modifiers(&mut scene, &brand, None);

    let node = scene.nodes.get(&1).unwrap();
    assert_eq!(
        node.raw.get("FontSize").and_then(|v| v.as_f64()),
        Some(30.0),
        "inline FontSize lands on the node"
    );
    assert_eq!(
        node.raw.get("__InlineFontSize").and_then(|v| v.as_bool()),
        Some(true),
        "inline FontSize is marked for font resolution precedence"
    );
}

/// Inline entries are the last cascade stage: a brand entry writing the same
/// field loses to the node's own inline style.
#[test]
fn inline_styles_override_brand_entries() {
    let mut scene = make_test_scene();
    scene.nodes.get_mut(&1).unwrap().raw = json!({
        "inlineStyles": [{
            "_Type_": "BuildingBlocks_StyleEntry",
            "conditionsList": [],
            "modifiers": [{
                "_Type_": "BuildingBlocks_FieldModifierNumber",
                "field": "FontSize",
                "value": 30.0
            }]
        }]
    });
    let brand_entries = [json!({
        "modifiers": [{
            "_Type_": "BuildingBlocks_FieldModifierNumber",
            "field": "FontSize",
            "value": 100.0
        }]
    })];
    let brand = BrandStyle {
        identifier: "test_brand".to_string(),
        entries: &brand_entries,
        raw: &json!({}),
    };

    apply_brand_modifiers(&mut scene, &brand, None);

    let node = scene.nodes.get(&1).unwrap();
    assert_eq!(
        node.raw.get("FontSize").and_then(|v| v.as_f64()),
        Some(30.0),
        "inline FontSize beats the brand entry"
    );
}
