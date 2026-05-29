//! Default value registry implementation.

use std::{collections::HashMap, sync::OnceLock};

use crate::canvas::{RgbaColor, Value};

#[derive(Debug, Clone, serde::Deserialize)]
struct WellKnownDefaultsData {
    paths: HashMap<String, Value>,
    localization: HashMap<String, String>,
}

fn well_known_defaults_data() -> &'static WellKnownDefaultsData {
    static DATA: OnceLock<WellKnownDefaultsData> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/default_value_registry_v1.json"))
            .expect("well-known default registry data must parse")
    })
}

/// Default values used to populate state-bound widgets when no live game data
/// is available.
#[derive(Debug, Default, Clone)]
pub struct DefaultValueRegistry {
    paths: HashMap<String, Value>,
    mfd_slot_defaults: HashMap<(u32, u32), String>,
    screen_state_colors: HashMap<String, RgbaColor>,
    localization: HashMap<String, String>,
}

impl DefaultValueRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry by combining well-known defaults and optional live localization.
    pub fn with_pipeline_defaults(localization_map: Option<HashMap<String, String>>) -> Self {
        let mut reg = Self::with_well_known_path_defaults();
        if let Some(loc_map) = localization_map
            && !loc_map.is_empty()
        {
            reg.merge_localization(loc_map);
        }
        reg
    }

    /// Create a registry pre-populated with well-known binding-path defaults.
    pub fn with_well_known_path_defaults() -> Self {
        let mut reg = Self::new();
        let data = well_known_defaults_data();
        for (path, value) in &data.paths {
            reg.insert_path(path, value.clone());
        }
        reg.set_localization(data.localization.clone());

        reg
    }

    /// Insert or overwrite a binding-path default.
    pub fn insert_path(&mut self, path: &str, default: Value) {
        self.paths.insert(path.to_owned(), default);
    }

    /// Insert or overwrite an MFD slot to content-canvas GUID mapping.
    pub fn insert_mfd_slot(&mut self, view: u32, slot: u32, content_guid: &str) {
        self.mfd_slot_defaults
            .insert((view, slot), content_guid.to_owned());
    }

    /// Look up a binding-path default.
    pub fn lookup_path(&self, path: &str) -> Option<&Value> {
        self.paths.get(path)
    }

    /// Look up the content-canvas GUID for an MFD screen slot.
    pub fn lookup_mfd_slot(&self, view: u32, slot: u32) -> Option<&str> {
        self.mfd_slot_defaults.get(&(view, slot)).map(String::as_str)
    }

    /// Ingest `screenStates[]` entries from parsed DataCore JSON.
    pub fn ingest_screen_states(&mut self, screen_states_json: &serde_json::Value) {
        let Some(states) = screen_states_json.as_array() else {
            return;
        };
        for state in states {
            let Some(name) = state["name"].as_str() else {
                continue;
            };
            let color = &state["color"];
            let Some(r) = color["r"].as_u64() else {
                continue;
            };
            let Some(g) = color["g"].as_u64() else {
                continue;
            };
            let Some(b) = color["b"].as_u64() else {
                continue;
            };
            let a = color["a"].as_u64().unwrap_or(255);
            self.screen_state_colors.insert(
                name.to_owned(),
                RgbaColor {
                    r: r as u8,
                    g: g as u8,
                    b: b as u8,
                    a: a as u8,
                },
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
    pub fn set_localization(&mut self, map: HashMap<String, String>) {
        self.localization = map;
    }

    /// Overlay `map` on top of the current localization table.
    pub fn merge_localization(&mut self, map: HashMap<String, String>) {
        for (key, value) in map {
            if self
                .localization
                .get(&key)
                .is_some_and(|existing| existing.is_empty())
            {
                continue;
            }
            self.localization.insert(key, value);
        }
    }

    /// Force-insert a single localization key with `value`.
    pub fn insert_localization(&mut self, key: &str, value: String) {
        self.localization.insert(key.to_owned(), value);
    }

    /// Look up a localization key.
    pub fn lookup_localization(&self, key: &str) -> Option<&str> {
        let bare = key.strip_prefix('@').unwrap_or(key);
        self.localization.get(&bare.to_lowercase()).map(String::as_str)
    }
}
