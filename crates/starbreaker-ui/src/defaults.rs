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
/// Four tables are maintained:
/// 1. **`paths`** — binding-path → [`Value`] for text/numeric defaults.
/// 2. **`mfd_slot_defaults`** — `(view_index, slot_index)` → content-canvas
///    GUID for MFD page selection.
/// 3. **`screen_state_colors`** — state-name → backlight [`RgbaColor`] from
///    `SCItemDisplayScreenComponentParams.screenStates[]`.
/// 4. **`localization`** — lowercase localization key → display string from
///    `global.ini` (e.g. `"hud_notarget"` → `"NO TARGET"`).
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

    /// Localization key (lowercase, without leading `@`) → display string.
    ///
    /// Populated from `Data\Localization\english\global.ini` when P4K access
    /// is available.  Used to resolve `labelProperties.label` fields on
    /// `WidgetTextField` nodes (e.g. `@hud_NoTarget` → `"NO TARGET"`).
    localization: HashMap<String, String>,
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

        // Med-bed header defaults (Clipper / hospital screens):
        // runtime `MedicalTier` commonly resolves to 3 in the static "switched-on"
        // view; operations add +1 to the min tier, so seed inputs as 2.
        reg.insert_path("CloneLocationInfo/MedicalTier", Value::Int(2));
        reg.insert_path("Bed/MedBed/MedBedStatus/MedicalTier", Value::Int(2));
        // Header medgel counters (top-right gauge and 200/200 label cluster).
        reg.insert_path("Bed/MedBed/MedBedStatus/containerOccupancy", Value::Int(200));
        reg.insert_path("Bed/MedBed/MedBedStatus/containerCapacity", Value::Int(200));
        // Header location binding used by the med-bed kiosk title row.
        reg.insert_path(
            "CloneLocationInfo/CurrentLocation/LocationName",
            Value::Str("Drake Clipper".into()),
        );
        // Base-screen state flags used by medical header title switching.
        reg.insert_path("state.BaseScreens.Heal", Value::Bool(false));
        reg.insert_path("state.BaseScreens.PerformingSurgery", Value::Bool(false));
        reg.insert_path("state.BaseScreens.ConfirmMoreInjuries", Value::Bool(false));
        reg.insert_path("state.BaseScreens.ConfirmNoInjuries", Value::Bool(false));
        reg.insert_path("state.BaseScreens.CloneMe", Value::Bool(false));
        reg.insert_path("state.BaseScreens.Admin", Value::Bool(false));
        // Popup state controls top-right close button visibility.
        reg.insert_path("Popup/IsActive", Value::Bool(true));

        // ── Static localization fallback ─────────────────────────────────────
        //
        // `Data\Localization\english\global.ini` is NOT present in the SC 4.x
        // P4K.  These built-in translations cover the keys used by the MFD BB
        // canvases so that `WidgetTextField` labels resolve to display text.
        //
        // Keys are lowercase (the format `lookup_localization` expects).
        // Values are the canonical English UI strings visible in-game.
        // If a live `global.ini` is ever loaded via `set_localization`, those
        // values take precedence (set_localization replaces the whole table).
        let mut loc: HashMap<String, String> = HashMap::new();
        // Target-status screen labels (gen_mc_s_target)
        loc.insert("hud_notarget".into(), "NO TARGET".into());
        loc.insert("dfm_ui_target".into(), "TARGET".into());
        loc.insert("hud_label_velocity".into(), "VELOCITY".into());
        loc.insert("scan_data_faction".into(), "FACTION".into());
        loc.insert("ui_nodata".into(), "—".into());
        loc.insert("innerthought_hail".into(), "HAIL".into());
        loc.insert("panel_call".into(), "CALL".into());
        // Weapon-info screen labels (gen_mc_s_weaponinfo)
        loc.insert("hud_gimbalmode".into(), "GIMBAL MODE".into());
        loc.insert("hud_activegroup".into(), "ACTIVE GROUP".into());
        // Annunciator chiclet labels (h_eng_annunciator + paramInputValues)
        loc.insert("hud_pwr".into(), "PWR".into());
        loc.insert("flighthud_label_wpn".into(), "WPN".into());
        loc.insert("hud_thr".into(), "THR".into());
        loc.insert("hud_shld".into(), "SHLD".into());
        loc.insert("hud_cool".into(), "COOL".into());
        // Self-status / target-status screen headers
        loc.insert("hud_selfstatus".into(), "SELF STATUS".into());
        loc.insert("hud_targetstatus".into(), "TARGET STATUS".into());
        // Emissions screen labels (gen_mc_s_emissions)
        loc.insert("hud_label_ir".into(), "IR".into());
        loc.insert("hud_label_em".into(), "EM".into());
        loc.insert("hud_label_cs".into(), "CS".into());
        loc.insert("hud_ir".into(), "IR:".into());
        loc.insert("hud_em".into(), "EM:".into());
        loc.insert("hud_cs".into(), "CS:".into());
        // Power output screen labels (gen_mc_s_poweroutputinfo)
        loc.insert("engineering_ui_item_output".into(), "OUTPUT".into());
        loc.insert("item_typebattery".into(), "BATTERY".into());
        loc.insert("hud_mode".into(), "MODE".into());
        loc.insert("hud_label_max".into(), "MAX".into());
        // Weapon-info label (gen_mc_s_weaponinfo — key present only as plural ,P form
        // in global.ini, so must also be in the static table for P4K-absent builds).
        loc.insert("hud_label_weapon".into(), "Weapon".into());
        // Misc HUD strings
        loc.insert("rn_offline".into(), "OFFLINE".into());
        loc.insert("rn_celsiussymbol".into(), "°C".into());
        // Intentionally empty / placeholder keys
        loc.insert("loc_empty".into(), String::new());
        loc.insert("loc_placeholder".into(), String::new());

        reg.set_localization(loc);

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

    /// Replace the localization table with `map`.
    ///
    /// Keys must be lowercase and must **not** carry a leading `@`.  This is
    /// the format produced by `palette::load_localization_map`.
    pub fn set_localization(&mut self, map: HashMap<String, String>) {
        self.localization = map;
    }

    /// Overlay `map` on top of the current localization table.
    ///
    /// Existing keys that are absent from `map` (e.g. the well-known
    /// fallbacks) are preserved.  Keys present in `map` override the
    /// current value.  This is the correct merge strategy when loading a
    /// partial `global.ini`: live data takes precedence, but every
    /// well-known fallback remains available for keys that the live file
    /// does not define.
    pub fn merge_localization(&mut self, map: HashMap<String, String>) {
        self.localization.extend(map);
    }

    /// Force-insert a single localization key with `value`, overriding any
    /// existing entry (including values loaded from global.ini).
    ///
    /// `key` must be lowercase and must **not** carry a leading `@`.
    pub fn insert_localization(&mut self, key: &str, value: String) {
        self.localization.insert(key.to_owned(), value);
    }

    /// Look up a localization key.
    ///
    /// `key` may start with `@` (it is stripped automatically before lookup).
    /// The remainder is lowercased for a case-insensitive match.
    ///
    /// Returns `None` when the key is absent from the table.
    pub fn lookup_localization(&self, key: &str) -> Option<&str> {
        let bare = key.strip_prefix('@').unwrap_or(key);
        self.localization.get(&bare.to_lowercase()).map(String::as_str)
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

    #[test]
    fn localization_lookup_with_at_prefix() {
        let mut reg = DefaultValueRegistry::new();
        let mut map = HashMap::new();
        map.insert("hud_notarget".to_owned(), "NO TARGET".to_owned());
        map.insert("dfm_ui_target".to_owned(), "TARGET".to_owned());
        reg.set_localization(map);

        // Key with '@' prefix, canonical uppercase source key.
        assert_eq!(reg.lookup_localization("@hud_NoTarget"), Some("NO TARGET"));
        // Key with '@' prefix, lowercase.
        assert_eq!(reg.lookup_localization("@hud_notarget"), Some("NO TARGET"));
        // Key without '@' prefix.
        assert_eq!(reg.lookup_localization("dfm_ui_target"), Some("TARGET"));
        // Missing key returns None.
        assert_eq!(reg.lookup_localization("@nonexistent"), None);
    }

    #[test]
    fn localization_lookup_empty_table_returns_none() {
        // new() creates a registry with no localization entries.
        let reg = DefaultValueRegistry::new();
        assert_eq!(reg.lookup_localization("@hud_NoTarget"), None);
    }

    #[test]
    fn well_known_defaults_has_localization_fallbacks() {
        // with_well_known_path_defaults() must include the static HUD strings.
        let reg = DefaultValueRegistry::with_well_known_path_defaults();
        assert_eq!(reg.lookup_localization("@hud_NoTarget"), Some("NO TARGET"));
        assert_eq!(reg.lookup_localization("@hud_GimbalMode"), Some("GIMBAL MODE"));
        assert_eq!(reg.lookup_localization("@hud_ActiveGroup"), Some("ACTIVE GROUP"));
        assert_eq!(reg.lookup_localization("@LOC_EMPTY"), Some(""));
        assert_eq!(reg.lookup_localization("@nonexistent"), None);
    }
}
