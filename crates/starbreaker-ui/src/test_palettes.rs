//! Test-only brand palettes loaded from the provenance fixture
//! `tests/fixtures/ui_ir/brand_palettes_v1.json` (extracted from the live
//! DataCore `BuildingBlocks_Style` records — see the adjacent `.notes.md`).
//!
//! Hard-coding palette values in test source is banned
//! (`rgba_colour_literals_are_not_hardcoded` guard; AGENTS.md Core rules):
//! fixtures that need REAL brand colours load them here so the values keep a
//! single, game-data-derived source of truth. The guard suite re-validates
//! the fixture against the live records whenever game data is present.

use crate::canvas::RgbaColor;
use crate::style::{CrtParams, ManufacturerStyle};

const FIXTURE: &str = include_str!("../tests/fixtures/ui_ir/brand_palettes_v1.json");

/// The verbatim `colorStyles` palette of a brand record, in `BB_ColorStyle`
/// enum order. Panics on unknown brands or authored-null slots — extend the
/// fixture (with provenance) rather than inventing values.
pub(crate) fn brand_colour_slots(brand: &str) -> Vec<RgbaColor> {
    let fixture: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("brand_palettes_v1.json parses");
    let slots = fixture["brands"][brand]["colorStyles"]
        .as_array()
        .unwrap_or_else(|| panic!("brand '{brand}' missing from brand_palettes_v1.json"));
    slots
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            let get = |k: &str| {
                slot.get(k)
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| panic!("brand '{brand}' slot {i} is null/invalid"))
                    as u8
            };
            RgbaColor { r: get("r"), g: get("g"), b: get("b"), a: get("a") }
        })
        .collect()
}

/// One palette slot by `BB_ColorStyle` enum index.
pub(crate) fn brand_slot(brand: &str, index: usize) -> RgbaColor {
    let slots = brand_colour_slots(brand);
    slots
        .get(index)
        .copied()
        .unwrap_or_else(|| panic!("brand '{brand}' has no slot {index}"))
}

/// A `ManufacturerStyle` assembled from the fixture palette the same way
/// `StyleLoader::parse_buildingblocks_style_record` assembles one from the
/// live record (primary = slot 0, background = slot 9, backlight = slot 11;
/// background slot adjudicated in plan P5.3 — see the loader).
pub(crate) fn brand_style(brand: &str) -> ManufacturerStyle {
    let slots = brand_colour_slots(brand);
    let name = brand.trim_start_matches("s_").split('_').next().unwrap_or(brand);
    ManufacturerStyle {
        name: name.to_string(),
        primary_tint: slots[0],
        secondary_tint: None,
        colour_slots: slots.clone(),
        background: slots[9],
        backlight: slots[11],
        font_family_hints: Vec::new(),
        crt: CrtParams::default(),
    }
}
