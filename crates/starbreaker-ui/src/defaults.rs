//! Default "switched on" state values for game-state-bound widgets.
//!
//! [`DefaultValueRegistry`] is the single authoritative source of literal
//! default text for runtime-bound UI widgets.  Every entry is keyed by the
//! **DataCore binding path** (e.g. `/vehicle/targetname`) — never by helper
//! name, mesh name, or ship name.  The companion `mfd_slot_defaults` map
//! stores per-slot content-canvas GUIDs for MFD page selection; the
//! `screen_state_colors` map stores the per-state backlight colors ingested
//! from `SCItemDisplayScreenComponentParams.screenStates[]`.
//!
//! # Authoritative source
//! The literal-default table is derived from the **Phase 1 Default-on contract
//! table** in `docs/ui-plan2.md`.  Each entry in
//! [`DefaultValueRegistry::with_well_known_path_defaults`] cites the binding
//! path that justifies it.

use std::collections::HashMap;

use crate::canvas::{RgbaColor, Value};

/// A registry of default values used to populate state-bound widgets when no
/// live game data is available ("switched on, no live data" state).
///
/// Three tables are maintained:
/// 1. **`paths`** — binding-path → [`Value`] for text/numeric defaults.
/// 2. **`mfd_slot_defaults`** — `(view_index, slot_index)` → content-canvas
///    GUID for MFD page selection.
/// 3. **`screen_state_colors`** — state-name → backlight [`RgbaColor`] from
///    `SCItemDisplayScreenComponentParams.screenStates[]`.
#[derive(Debug, Default, Clone)]
pub struct DefaultValueRegistry {
    /// Binding-path → default [`Value`].
    ///
    /// Path format mirrors what BuildingBlocks Operations use, e.g.
    /// `"/vehicle/targetname"`, `"/ship/hp/current"`.
    paths: HashMap<String, Value>,

    /// Per-slot MFD content-canvas GUID default.
    ///
    /// Key: `(dashboard_view_index, dashboard_screen_slot)`.  This is the
    /// *slot position* pair from the dashboard canvas config, **not** the
    /// helper mesh name, keeping the registry ship-agnostic.
    mfd_slot_defaults: HashMap<(u32, u32), String>,

    /// Per-screen-state-name → backlight SRGBA color.
    ///
    /// Ingested from `SCItemDisplayScreenComponentParams.screenStates[]`.
    /// Note: this is the *backlight glow* color (e.g. cyan for Drake screens),
    /// **not** the amber UI-content tint which comes from the CRT shader.
    screen_state_colors: HashMap<String, RgbaColor>,
}

impl DefaultValueRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry pre-populated with the well-known binding-path
    /// defaults from the Phase 1 Default-on contract table.
    ///
    /// Every entry here must cite its binding path.  Ship names, helper names,
    /// and mesh names must **not** appear as keys.
    pub fn with_well_known_path_defaults() -> Self {
        let mut reg = Self::new();

        // /vehicle/targetname — Target Status MFD: the "no target locked" text.
        // (Phase 1 contract: "NO TARGET when no target selected")
        reg.insert_path("/vehicle/targetname", Value::Str("NO TARGET".into()));

        // /vehicle/target/distance — Radar/Target footer range label.
        // (Phase 1 contract: Radar range default "0.0km")
        reg.insert_path("/vehicle/target/distance", Value::Str("0.0km".into()));

        // /vehicle/target/bearing — Radar/Target footer heading label.
        // (Phase 1 contract: Radar heading default "0°")
        reg.insert_path("/vehicle/target/bearing", Value::Str("0°".into()));

        // /ship/hp/current — Self Status MFD HP box current value.
        // (Phase 1 contract: HP binding TBD in Phase 7; use "MAX" as placeholder)
        reg.insert_path("/ship/hp/current", Value::Str("MAX".into()));

        // /ship/hp/max — Self Status MFD HP box maximum value.
        // (Phase 1 contract: same TBD caveat; use "MAX" as placeholder)
        reg.insert_path("/ship/hp/max", Value::Str("MAX".into()));

        // /seatdashboard/powerstate — Power Management MFD battery state label.
        // (Phase 1 contract: "0 / 0 OFFLINE" battery; "OFFLINE" is the state string)
        reg.insert_path("/seatdashboard/powerstate", Value::Str("OFFLINE".into()));

        // /seatdashboard/powercurrent — Power Management MFD output numerator.
        // (Phase 1 contract: "2 / 16" power output observed in reference)
        reg.insert_path("/seatdashboard/powercurrent", Value::Int(2));

        // /seatdashboard/powermax — Power Management MFD output denominator.
        // (Phase 1 contract: "2 / 16" power output observed in reference)
        reg.insert_path("/seatdashboard/powermax", Value::Int(16));

        // /vehicle/gungroup — Self Status MFD top-right label.
        // (Phase 1 contract: "GUNS (ALL)" gun-group runtime default)
        reg.insert_path("/vehicle/gungroup", Value::Str("(ALL)".into()));

        reg
    }

    /// Insert or overwrite a binding-path default.
    pub fn insert_path(&mut self, path: &str, default: Value) {
        self.paths.insert(path.to_owned(), default);
    }

    /// Insert or overwrite an MFD slot → content-canvas GUID mapping.
    ///
    /// `view` and `slot` are the `dashboard_view_index` and
    /// `dashboard_screen_slot` values from the dashboard canvas config.
    /// `content_guid` is the DataCore GUID of the MFD page canvas.
    pub fn insert_mfd_slot(&mut self, view: u32, slot: u32, content_guid: &str) {
        self.mfd_slot_defaults.insert((view, slot), content_guid.to_owned());
    }

    /// Look up a binding-path default.
    ///
    /// Returns `None` if no default has been registered for `path`.
    pub fn lookup_path(&self, path: &str) -> Option<&Value> {
        self.paths.get(path)
    }

    /// Look up the content-canvas GUID for an MFD screen slot.
    ///
    /// Returns `None` if no default has been registered for `(view, slot)`.
    pub fn lookup_mfd_slot(&self, view: u32, slot: u32) -> Option<&str> {
        self.mfd_slot_defaults.get(&(view, slot)).map(String::as_str)
    }

    /// Ingest `screenStates[]` entries from a parsed DataCore record JSON.
    ///
    /// Walks `record_json["screenStates"]` (an array).  Each element must
    /// contain a `"name"` string and a `"color"` object with `"r"`, `"g"`,
    /// `"b"`, `"a"` integer fields (SRGBA8).  Elements that do not match this
    /// shape are silently skipped.
    ///
    /// The resulting colors are stored in [`screen_state_colors`] under the
    /// state name (e.g. `"Normal"`).  These represent the screen *backlight*
    /// color (typically cyan for Drake vehicles), **not** the amber UI-content
    /// tint.
    pub fn ingest_screen_states(&mut self, screen_states_json: &serde_json::Value) {
        let Some(states) = screen_states_json.as_array() else {
            return;
        };
        for state in states {
            let Some(name) = state["name"].as_str() else {
                continue;
            };
            let color = &state["color"];
            let Some(r) = color["r"].as_u64() else { continue };
            let Some(g) = color["g"].as_u64() else { continue };
            let Some(b) = color["b"].as_u64() else { continue };
            let a = color["a"].as_u64().unwrap_or(255);
            self.screen_state_colors.insert(
                name.to_owned(),
                RgbaColor { r: r as u8, g: g as u8, b: b as u8, a: a as u8 },
            );
        }
    }

    /// Return the number of binding-path defaults currently registered.
    pub fn path_count(&self) -> usize {
        self.paths.len()
    }

    /// Return the backlight color registered for a screen state name, if any.
    pub fn screen_state_color(&self, state_name: &str) -> Option<RgbaColor> {
        self.screen_state_colors.get(state_name).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Value;

    #[test]
    fn well_known_defaults_has_at_least_nine_entries() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert!(
            reg.path_count() >= 9,
            "expected ≥9 well-known path defaults, got {}",
            reg.path_count()
        );
    }

    #[test]
    fn well_known_defaults_targetname() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(
            reg.lookup_path("/vehicle/targetname"),
            Some(&Value::Str("NO TARGET".into()))
        );
    }

    #[test]
    fn well_known_defaults_target_distance() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(
            reg.lookup_path("/vehicle/target/distance"),
            Some(&Value::Str("0.0km".into()))
        );
    }

    #[test]
    fn well_known_defaults_target_bearing() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(
            reg.lookup_path("/vehicle/target/bearing"),
            Some(&Value::Str("0°".into()))
        );
    }

    #[test]
    fn well_known_defaults_hp() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(reg.lookup_path("/ship/hp/current"), Some(&Value::Str("MAX".into())));
        assert_eq!(reg.lookup_path("/ship/hp/max"), Some(&Value::Str("MAX".into())));
    }

    #[test]
    fn well_known_defaults_power() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(
            reg.lookup_path("/seatdashboard/powerstate"),
            Some(&Value::Str("OFFLINE".into()))
        );
        assert_eq!(reg.lookup_path("/seatdashboard/powercurrent"), Some(&Value::Int(2)));
        assert_eq!(reg.lookup_path("/seatdashboard/powermax"), Some(&Value::Int(16)));
    }

    #[test]
    fn well_known_defaults_gungroup() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(
            reg.lookup_path("/vehicle/gungroup"),
            Some(&Value::Str("(ALL)".into()))
        );
    }

    #[test]
    fn well_known_defaults_unknown_path_returns_none() {
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(reg.lookup_path("/nonexistent/binding"), None);
    }

    #[test]
    fn mfd_slot_roundtrip() {
        let mut reg = DefaultValueRegistry::new();
        reg.insert_mfd_slot(4, 6, "3228e5cc-d0b7-49f7-9765-20cbad1b61b0");
        assert_eq!(
            reg.lookup_mfd_slot(4, 6),
            Some("3228e5cc-d0b7-49f7-9765-20cbad1b61b0")
        );
        assert_eq!(reg.lookup_mfd_slot(4, 7), None);
    }

    #[test]
    fn ingest_screen_states_parses_normal_state() {
        // Fixture: one "Normal" state with the documented Drake screen backlight
        // color: SRGBA8(r:102, g:214, b:255, a:255).
        // Source: Phase 1 findings — SCItemDisplayScreenComponentParams.screenStates[]
        // "Normal": lightOn=true, color=SRGBA8(r:102,g:214,b:255,a:255), intensity=0.025
        let fixture = serde_json::json!([
            {
                "name": "Normal",
                "lightOn": true,
                "color": { "r": 102, "g": 214, "b": 255, "a": 255 },
                "intensity": 0.025
            }
        ]);
        let mut reg = DefaultValueRegistry::new();
        reg.ingest_screen_states(&fixture);
        assert_eq!(
            reg.screen_state_color("Normal"),
            Some(RgbaColor { r: 102, g: 214, b: 255, a: 255 })
        );
        assert_eq!(reg.screen_state_color("Unknown"), None);
    }

    #[test]
    fn ingest_screen_states_skips_malformed_entries() {
        // Entry missing "color" sub-object should be skipped without panic.
        let fixture = serde_json::json!([
            { "name": "Broken" },
            { "name": "Good", "color": { "r": 10, "g": 20, "b": 30, "a": 255 } }
        ]);
        let mut reg = DefaultValueRegistry::new();
        reg.ingest_screen_states(&fixture);
        assert_eq!(reg.screen_state_color("Broken"), None);
        assert_eq!(
            reg.screen_state_color("Good"),
            Some(RgbaColor { r: 10, g: 20, b: 30, a: 255 })
        );
    }

    #[test]
    fn ingest_screen_states_non_array_is_no_op() {
        let mut reg = DefaultValueRegistry::new();
        reg.ingest_screen_states(&serde_json::json!("not an array"));
        assert_eq!(reg.path_count(), 0);
    }
}
