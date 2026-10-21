//! Stand-in for `ui_pipeline` when `starbreaker-3d` is built without the
//! `ui` feature.
//!
//! Mirrors the entry points `decomposed.rs` calls ([`UiShipData`],
//! [`UiLocData`], [`build_default_registry`], [`render_ui_binding_png`]) so the
//! export pipeline compiles unchanged. Screen rendering is unavailable:
//! [`render_ui_binding_png`] always returns an error, which the decomposed
//! export already treats as "no generated image" for that binding. GLB and
//! character exports do not touch this module.
//!
//! `screen_aspect` has no `starbreaker-ui` dependency and is shared with the
//! real module.

use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;

use crate::types::UiBinding;

#[path = "ui_pipeline/screen_aspect.rs"]
pub(crate) mod screen_aspect;

/// Per-ship UI values; empty without the `ui` feature.
pub struct UiShipData;

impl UiShipData {
    pub fn none() -> Self {
        Self
    }

    pub fn derive(_db: &Database<'_>, _root_entity_name: &str) -> Self {
        Self
    }
}

/// Localization data for UI renders; nothing is loaded without the `ui` feature.
pub struct UiLocData;

impl UiLocData {
    pub fn load(_p4k: &MappedP4k) -> Self {
        Self
    }
}

/// Placeholder for `starbreaker_ui::DefaultValueRegistry`.
pub struct DefaultValueRegistry;

pub fn build_default_registry(
    _loc_data: &UiLocData,
    _ship_data: &UiShipData,
) -> DefaultValueRegistry {
    DefaultValueRegistry
}

/// Always fails: screen rendering needs the `ui` feature.
pub fn render_ui_binding_png(
    _binding: &UiBinding,
    _db: &Database<'_>,
    _p4k: &MappedP4k,
    _texture_mip: u32,
    _root_manufacturer_id: Option<&str>,
    _loc_data: &UiLocData,
    _ship_data: &UiShipData,
    _defaults_registry: Option<&DefaultValueRegistry>,
    _root_entity_name: Option<&str>,
) -> Result<Vec<u8>, String> {
    Err("starbreaker-3d was built without the `ui` feature".to_string())
}
