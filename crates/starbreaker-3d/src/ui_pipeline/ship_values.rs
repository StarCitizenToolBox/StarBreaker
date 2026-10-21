//! Per-ship UI binding values derived from DataCore (power-screen data).
//!
//! [`UiShipData`] is per-export shared state (like `UiLocData`): built once
//! from the root vehicle's records and passed to every UI binding render as
//! `PipelineInputs::derived_values`. It derives the power-management screen's
//! at-rest model from game data instead of the static registry pins:
//!
//! - pools → pip stacks: `Components[SItemPortContainerComponentParams]
//!   .resourceNetworkPowerPools.itemPools`; a `FixedPowerPool` contributes
//!   `poolSize` pips, a `DynamicPowerPool` the sum of its fitted items'
//!   Online-state `SPowerSegmentResourceUnit.units` power consumption;
//! - in-use pips: pool total × `ItemResourceNetworkGlobal
//!   .defaultPowerDistributionParams.poolDefault{Weapons,Engines,Shields}`;
//! - per-pool icons: pool `itemType` → `ItemResourceNetworkGlobal
//!   .uiParams.typeData[enum index].typeIcon` (gun = three bullets,
//!   thrusters = chevrons, shield generator = shield in a dashed circle —
//!   all verified against the in-game reference);
//! - heat gauge: shown for pools whose fitted items model temperature;
//!   `overheatTemperature` comes from the item's
//!   `EntityTemperatureItemResource`, the remaining scale values are
//!   documented at-rest constants (see `AMBIENT_TEMP_K` etc.).
//!
//! NOT yet derived (static registry pins remain authoritative): OUTPUT
//! `availablePower`/`totalPossiblePower` (the 2/16 formula is unresearched —
//! the two fitted plants generate 2×14=28 units ≠ 16) and the IR/EM/CS
//! emissions header values.

use std::collections::HashMap;

use serde_json::Value as Json;
use starbreaker_datacore::loadout::{resolve_loadout_indexed, EntityIndex};
use starbreaker_datacore::Database;
use starbreaker_ui::canvas::Value as UiValue;

/// At-rest ambient temperature shown on heat gauges (Kelvin). The engine
/// feeds live entity temperature here; statically the reference screenshot's
/// gauges sit at ambient.
const AMBIENT_TEMP_K: f64 = 290.0;
/// Heat-gauge scale floor (0 °C) — engine gauge minimum, not in item data.
const MIN_TEMP_K: f64 = 273.0;
/// Heat-gauge scale headroom above the overheat threshold (the critical
/// band). Calibrated from the reference shield gauge (scale top 440 K for
/// the 372 K overheat item); the engine's actual scale rule is unresearched,
/// so the ceiling tracks each item's real overheat coherently.
const OVERHEAT_HEADROOM_K: f64 = 68.0;
/// Overheat threshold fallback when a pool's items carry no temperature
/// model (matches the small-component `overheatTemperature` family).
const OVERHEAT_TEMP_K: f64 = 372.0;
/// At-rest compass heading (degrees). The flight controller is not simulated in
/// a static export, so the cockpit compass shows the neutral forward heading
/// (0° = North) — the same at-rest principle as the velocity / g-force = 0 pins,
/// NOT a value copied from the reference capture (the reference is a live in-flight
/// heading). Every projection number comes from `SVehicleHudParams.compassTape`.
const COMPASS_AT_REST_HEADING_DEG: f64 = 0.0;

/// Per-export UI values derived from the root vehicle's DataCore records.
pub struct UiShipData {
    /// Binding-path overrides for `PipelineInputs::derived_values`. `None`
    /// when the root entity has no resource-network power pools (non-vehicle
    /// exports keep the static registry).
    pub derived_values: Option<HashMap<String, UiValue>>,
}

impl UiShipData {
    /// No derived data (static registry defaults apply).
    pub fn none() -> Self {
        Self { derived_values: None }
    }

    /// The radar/compass heading (degrees) the radar plane chrome rotates by — the
    /// SHARED `FlightController/Compass/Value` (the same source as the cockpit
    /// compass tape + the radar heading readout; 0 at static rest). Driving the
    /// radar plane's heading-up rotation from this keeps the spokes, heading ring
    /// and cardinal markers rotating WITH the ship heading.
    pub fn radar_heading_deg(&self) -> f32 {
        self.derived_values
            .as_ref()
            .and_then(|m| m.get("FlightController/Compass/Value"))
            .map(|v| match v {
                UiValue::Int(i) => *i as f32,
                UiValue::Float(f) => *f as f32,
                _ => 0.0,
            })
            .unwrap_or(0.0)
    }

    /// Derive per-ship UI values from `root_entity_name`'s records.
    pub fn derive(db: &Database<'_>, root_entity_name: &str) -> Self {
        let idx = EntityIndex::new(db);
        let stem = root_entity_name.rsplit('.').next().unwrap_or(root_entity_name);
        let Some(record) = idx.find_record(stem) else {
            return Self::none();
        };
        let Some(vehicle_json) = record_json(db, record) else {
            return Self::none();
        };
        let mut overrides: HashMap<String, UiValue> = HashMap::new();

        // Power-management screen values, only when the vehicle has power pools.
        let pools = power_pools_from_vehicle(&vehicle_json);
        if !pools.is_empty() {
            // Top-level fitted items (controllers, shields, plants …), each
            // materialised once: dynamic pools consume their power units.
            let tree = resolve_loadout_indexed(&idx, record);
            let mut item_cache: HashMap<String, Option<Json>> = HashMap::new();
            let mut fitted_items: Vec<Json> = Vec::new();
            for child in &tree.root.children {
                let entry = item_cache
                    .entry(child.entity_name.to_ascii_lowercase())
                    .or_insert_with(|| record_json(db, &child.record));
                if let Some(json) = entry.clone() {
                    fitted_items.push(json);
                }
            }
            let pool_defaults = pool_defaults_from_global(db);
            let pool_icons = pool_icons_from_global(db);
            overrides.extend(derive_power_paths(
                &pools,
                &fitted_items,
                &pool_defaults,
                &pool_icons,
            ));
        }

        // Cockpit compass tape: the heading-tick array (`FlightController/Compass/
        // Ticks`) is engine-runtime data the flight HUD fills live. The static
        // export has no flight controller, so derive the AT-REST tick set from the
        // global vehicle-HUD params (visible degree range / major increment /
        // sub-ticks) at the neutral forward heading — same at-rest principle as the
        // velocity / g-force = 0 pins. Overrides the static `…/Ticks = 0` fallback.
        if let Some(tape) = compass_tape_from_global(db) {
            overrides.extend(derive_compass_ticks(&tape, COMPASS_AT_REST_HEADING_DEG));
        }

        // Cockpit-compass heading VALUE — the SINGLE shared heading source for the
        // cockpit compass tape AND the radar readout's heading text
        // (`Text_RadarRotation` binds `FlightController/Compass/Value`; the compass
        // tape centres on the same heading). Deriving it from the one
        // `COMPASS_AT_REST_HEADING_DEG` constant keeps the two readouts in
        // agreement — changing the heading moves both. At rest the flight
        // controller isn't simulated, so this is the neutral forward heading (0°),
        // the same at-rest principle as the tick set above.
        overrides.insert(
            "FlightController/Compass/Value".to_string(),
            UiValue::Int(COMPASS_AT_REST_HEADING_DEG as i64),
        );

        // Cockpit countermeasure list: the `List_Countermeasures` widget binds the
        // engine-state array `WeaponController/Countermeasures/Launchers` (one entry
        // per fitted launcher, each showing its loaded ammo). The static export has
        // no weapon controller, so derive the at-rest list from the vehicle's fitted
        // countermeasure launchers — same at-rest principle as the compass/velocity
        // pins (the in-game reference may show a partially-fired live count).
        overrides.extend(derive_countermeasure_launchers(db, &idx, record));

        if overrides.is_empty() {
            return Self::none();
        }
        if std::env::var("SB_SHIP_VALUES_DUMP").as_deref() == Ok("1") {
            let mut entries: Vec<_> = overrides.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (path, value) in entries {
                eprintln!("[ship-values] {path} = {value:?}");
            }
        }
        Self { derived_values: Some(overrides) }
    }
}

/// A vehicle power pool: `FixedPowerPool` carries an authored size, a
/// `DynamicPowerPool` is sized by its fitted items' consumption.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PowerPool {
    pub item_type: String,
    pub fixed_size: Option<u32>,
}

/// Default at-rest power assignment fractions
/// (`ItemResourceNetworkGlobal.defaultPowerDistributionParams`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolDefaults {
    pub weapons: f64,
    pub engines: f64,
    pub shields: f64,
}

impl Default for PoolDefaults {
    fn default() -> Self {
        Self { weapons: 0.5, engines: 0.5, shields: 0.5 }
    }
}

fn record_json(db: &Database<'_>, record: &starbreaker_datacore::types::Record) -> Option<Json> {
    let bytes = starbreaker_datacore::export::to_json_compact(db, record).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Component entry of `_Type_` `ty` from a record JSON (`_RecordValue_`
/// wrapper tolerated).
fn component<'a>(record_json: &'a Json, ty: &str) -> Option<&'a Json> {
    record_json
        .get("_RecordValue_")
        .unwrap_or(record_json)
        .get("Components")?
        .as_array()?
        .iter()
        .find(|c| c.get("_Type_").and_then(|v| v.as_str()) == Some(ty))
}

pub(crate) fn power_pools_from_vehicle(vehicle_json: &Json) -> Vec<PowerPool> {
    let Some(pools) = component(vehicle_json, "SItemPortContainerComponentParams")
        .and_then(|c| c.get("resourceNetworkPowerPools"))
        .and_then(|p| p.get("itemPools"))
        .and_then(|p| p.as_array())
    else {
        return Vec::new();
    };
    pools
        .iter()
        .filter_map(|pool| {
            let item_type = pool.get("itemType")?.as_str()?.to_string();
            let fixed_size = (pool.get("_Type_").and_then(|v| v.as_str())
                == Some("FixedPowerPool"))
            .then(|| pool.get("poolSize").and_then(|v| v.as_u64()).unwrap_or(0) as u32);
            Some(PowerPool { item_type, fixed_size })
        })
        .collect()
}

/// `AttachDef.Type` of a fitted item.
pub(crate) fn item_attach_type(item_json: &Json) -> Option<String> {
    component(item_json, "SAttachableComponentParams")?
        .get("AttachDef")?
        .get("Type")?
        .as_str()
        .map(str::to_string)
}

/// Online-state power consumption in `SPowerSegmentResourceUnit` units.
pub(crate) fn item_power_units(item_json: &Json) -> u32 {
    let Some(states) = component(item_json, "ItemResourceComponentParams")
        .and_then(|c| c.get("states"))
        .and_then(|s| s.as_array())
    else {
        return 0;
    };
    states
        .iter()
        .filter(|state| state.get("name").and_then(|v| v.as_str()) == Some("Online"))
        .flat_map(|state| state.get("deltas").and_then(|d| d.as_array()).into_iter().flatten())
        .filter_map(|delta| {
            let consumption = delta.get("consumption")?;
            if consumption.get("resource")?.as_str()? != "Power" {
                return None;
            }
            let amount = consumption.get("resourceAmountPerSecond")?;
            if amount.get("_Type_")?.as_str()? != "SPowerSegmentResourceUnit" {
                return None;
            }
            amount.get("units")?.as_u64()
        })
        .sum::<u64>() as u32
}

/// The item's overheat threshold from its entity temperature model.
pub(crate) fn item_overheat_temperature(item_json: &Json) -> Option<f64> {
    component(item_json, "SEntityPhysicsControllerParams")?
        .get("PhysType")?
        .get("temperature")?
        .get("itemResourceParams")?
        .get("overheatTemperature")?
        .as_f64()
}

fn pool_defaults_from_global(db: &Database<'_>) -> PoolDefaults {
    let Some(si) = db.struct_id("ItemResourceNetworkGlobal") else {
        return PoolDefaults::default();
    };
    let Some(json) = db
        .records_of_type(si)
        .next()
        .and_then(|record| record_json(db, record))
    else {
        return PoolDefaults::default();
    };
    let params = json
        .get("_RecordValue_")
        .unwrap_or(&json)
        .get("defaultPowerDistributionParams");
    let field = |name: &str, fallback: f64| {
        params
            .and_then(|p| p.get(name))
            .and_then(|v| v.as_f64())
            .unwrap_or(fallback)
    };
    PoolDefaults {
        weapons: field("poolDefaultWeapons", 0.5),
        engines: field("poolDefaultEngines", 0.5),
        shields: field("poolDefaultShields", 0.5),
    }
}

/// Cockpit compass "tape" projection, from `SVehicleHudParams.compassTape`.
/// `range` is the visible window in degrees (±range/2 about the heading);
/// `main_tick_increment` the spacing between LABELLED major ticks; `sub_ticks`
/// the number of sub-divisions per major interval (minor tick spacing =
/// `main_tick_increment / sub_ticks`). For the default HUD: 90° / 20° / 4 →
/// labelled ticks every 20°, minor ticks every 5° (verified against the
/// in-game `compass_master.png`: majors 20° apart, minors 5° apart).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CompassTape {
    pub range_deg: f64,
    pub main_tick_increment_deg: f64,
    pub sub_ticks: u32,
}

fn compass_tape_from_global(db: &Database<'_>) -> Option<CompassTape> {
    let si = db.struct_id("SVehicleHudParams")?;
    let json = db
        .records_of_type(si)
        .next()
        .and_then(|record| record_json(db, record))?;
    let tape = json
        .get("_RecordValue_")
        .unwrap_or(&json)
        .get("compassTape")?;
    Some(CompassTape {
        range_deg: tape.get("range")?.as_f64()?,
        main_tick_increment_deg: tape.get("mainTickIncrement")?.as_f64()?,
        sub_ticks: tape.get("subTicks")?.as_u64()? as u32,
    })
}

/// Build the at-rest compass `FlightController/Compass/Ticks` entry array from
/// the `compassTape` projection at `heading_deg`. Each engine tick entry carries
/// `anchor` (0..1 position across the strip), `maintick` (labelled-major flag),
/// and `value` (the degree shown). The materialised `WidgetList` reads the count
/// at the array path and each entry's fields at `…/Ticks/[000j]/<field>`.
pub(crate) fn derive_compass_ticks(tape: &CompassTape, heading_deg: f64) -> HashMap<String, UiValue> {
    let mut paths: HashMap<String, UiValue> = HashMap::new();
    if !(tape.range_deg > 0.0) || !(tape.main_tick_increment_deg > 0.0) || tape.sub_ticks == 0 {
        return paths;
    }
    let minor_deg = tape.main_tick_increment_deg / tape.sub_ticks as f64;
    let half = tape.range_deg / 2.0;
    let main_int = tape.main_tick_increment_deg.round() as i64;
    let base = "FlightController/Compass/Ticks";
    // Ticks fall on multiples of `minor_deg`; emit those inside the visible
    // window [heading-half, heading+half], left (low degree) to right (high).
    let first_k = ((heading_deg - half) / minor_deg).ceil() as i64;
    let last_k = ((heading_deg + half) / minor_deg).floor() as i64;
    let mut entry: i64 = 0;
    for k in first_k..=last_k {
        let degree = k as f64 * minor_deg;
        let anchor = 0.5 + (degree - heading_deg) / tape.range_deg;
        let degree_int = degree.round() as i64;
        let is_main = main_int != 0 && degree_int.rem_euclid(main_int) == 0;
        let value = degree_int.rem_euclid(360);
        paths.insert(format!("{base}/[{entry:04}]/anchor"), UiValue::Float(anchor));
        paths.insert(format!("{base}/[{entry:04}]/maintick"), UiValue::Bool(is_main));
        paths.insert(format!("{base}/[{entry:04}]/value"), UiValue::Int(value));
        entry += 1;
    }
    // The count the WidgetList reads at the array path (overrides the static
    // `…/Ticks = 0` empty-at-rest fallback for vehicles whose HUD params resolve).
    paths.insert(base.to_string(), UiValue::Int(entry));
    paths
}

/// Derive the at-rest cockpit countermeasure launcher list from the vehicle's
/// fitted launchers. One entry per `hardpoint_countermeasure*` loadout child, in
/// loadout order; the `List_Countermeasures` ColumnReverse flex lays index 0 at
/// the bottom panel and the next at the top.
fn derive_countermeasure_launchers(
    db: &Database<'_>,
    idx: &EntityIndex,
    record: &starbreaker_datacore::types::Record,
) -> HashMap<String, UiValue> {
    let tree = resolve_loadout_indexed(idx, record);
    let counts: Vec<i64> = tree
        .root
        .children
        .iter()
        // Generic discriminator: the item port category, never a ship/item name.
        .filter(|child| child.item_port_name.to_ascii_lowercase().contains("countermeasure"))
        .map(|child| {
            let json = record_json(db, &child.record);
            let Some(j) = json.as_ref() else {
                return 0;
            };
            let ammo = countermeasure_initial_ammo(j).unwrap_or(0);
            countermeasure_display_count(ammo, countermeasure_is_burst(j))
        })
        .collect();
    derive_countermeasure_paths(&counts)
}

/// A countermeasure launcher's full at-rest load:
/// `SAmmoContainerComponentParams.initialAmmoCount`.
fn countermeasure_initial_ammo(item_json: &Json) -> Option<i64> {
    component(item_json, "SAmmoContainerComponentParams")?
        .get("initialAmmoCount")?
        .as_i64()
}

/// Engine-runtime countermeasure CHARGE SIZE (flares deployed per full burst).
///
/// REFERENCE-DERIVED IOU (ui-workflow §8). The Clipper's 48-flare BEHR decoy
/// launcher reads "4" in-game (48 / 12), and firing a full batch decrements the
/// count by one (owner-observed). This divisor is NOT in any decodable record —
/// the launcher weapon component / fire-action (`shotCount` = 1) / the
/// `behr_flare` ammo params / the resource type / the vehicle weapon controller /
/// P4K all lack it, and a cross-ship survey (decoy `maxAmmoCount` 48/25/30) fits
/// no stored divisor — so the engine's countermeasure manager computes "batches
/// remaining" in C++. Pinned as a SINGLE UNIVERSAL constant (NOT per-ship: the
/// Single-vs-Burst fire-action type is the data discriminator; per-ship variation
/// comes from the differing ammo counts). TODO: replace with the decoded engine
/// charge/burst size once found.
const COUNTERMEASURE_CHARGE_SIZE: i64 = 12;

/// The at-rest count shown on a countermeasure launcher panel. A SINGLE-fire
/// launcher (noise) deploys one round per trigger, so the panel shows the raw
/// ammo. A BURST-fire launcher (decoy) deploys a charge of
/// [`COUNTERMEASURE_CHARGE_SIZE`] per full burst, so the panel shows the BATCHES
/// remaining = `ceil(ammo / charge)` (firing a partial batch leaves the ceil
/// unchanged; a full batch decrements it — matching the in-game behaviour).
fn countermeasure_display_count(initial_ammo: i64, is_burst: bool) -> i64 {
    if initial_ammo <= 0 {
        return 0;
    }
    if is_burst {
        (initial_ammo + COUNTERMEASURE_CHARGE_SIZE - 1) / COUNTERMEASURE_CHARGE_SIZE
    } else {
        initial_ammo
    }
}

/// True when the launcher's weapon component fires in BURST mode (decoy), false
/// for SINGLE mode (noise). The fire-action `_Type_`
/// (`SWeaponActionFireBurstParams` vs `SWeaponActionFireSingleParams`) is the
/// generic discriminator — never a ship/item name.
fn countermeasure_is_burst(item_json: &Json) -> bool {
    component(item_json, "SCItemWeaponComponentParams")
        .and_then(|w| w.get("fireActions"))
        .and_then(|v| v.as_array())
        .is_some_and(|actions| {
            actions.iter().any(|a| {
                a.get("_Type_").and_then(|v| v.as_str()) == Some("SWeaponActionFireBurstParams")
            })
        })
}

/// Build the countermeasure-list binding paths from each fitted launcher's
/// at-rest ammo (in loadout order). The `List_Countermeasures` arrayVariable
/// reads the count at the base path and clones the entry template per launcher
/// under `…/[NNNN]`; each entry binds `AmmoCount` (the full load) plus the
/// firing-state fields (`IsFiring`/`CurrentBurstSize`/`BurstSizeHoldRatio`),
/// which rest at zero so the hold-to-fire overlay stays hidden at rest.
pub(crate) fn derive_countermeasure_paths(ammo: &[i64]) -> HashMap<String, UiValue> {
    let mut paths: HashMap<String, UiValue> = HashMap::new();
    if ammo.is_empty() {
        return paths;
    }
    let base = "WeaponController/Countermeasures/Launchers";
    for (index, &count) in ammo.iter().enumerate() {
        let entry = format!("{base}/[{index:04}]");
        paths.insert(format!("{entry}/AmmoCount"), UiValue::Int(count));
        paths.insert(format!("{entry}/IsFiring"), UiValue::Bool(false));
        paths.insert(format!("{entry}/CurrentBurstSize"), UiValue::Int(0));
        paths.insert(format!("{entry}/BurstSizeHoldRatio"), UiValue::Float(0.0));
    }
    paths.insert(base.to_string(), UiValue::Int(ammo.len() as i64));
    paths
}

/// Canonical display order of the power-screen system list: the engine
/// presents weapons, engines, shields first (the
/// `defaultPowerDistributionParams` W/E/S order, matching the in-game
/// reference), then the remaining pools in authored order.
fn pool_display_rank(item_type: &str) -> u8 {
    match item_type {
        "WeaponGun" => 0,
        "FlightController" => 1,
        "Shield" => 2,
        _ => 3,
    }
}

fn pool_default_fraction(item_type: &str, defaults: &PoolDefaults) -> f64 {
    match item_type {
        "WeaponGun" => defaults.weapons,
        "FlightController" => defaults.engines,
        "Shield" => defaults.shields,
        _ => 0.0,
    }
}

/// Pool `itemType` → power-screen glyph from
/// `ItemResourceNetworkGlobal.uiParams.typeData`, indexed by the DataCore
/// item-type enum (the array position IS the enum value — verified against
/// the in-game reference: gun = three bullets, thrusters = chevrons, shield
/// generator = shield in a dashed circle).
fn pool_icons_from_global(db: &Database<'_>) -> HashMap<String, String> {
    let mut icons = HashMap::new();
    let Some(options) = (0..db.enum_defs().len() as i32).find_map(|i| {
        let names: Vec<&str> = db
            .enum_options(i)
            .iter()
            .map(|id| db.resolve_string2(*id))
            .collect();
        (names.iter().any(|n| *n == "WeaponGun")
            && names.iter().any(|n| *n == "FlightController"))
        .then_some(names)
    }) else {
        return icons;
    };
    let Some(type_data) = db
        .struct_id("ItemResourceNetworkGlobal")
        .and_then(|si| db.records_of_type(si).next())
        .and_then(|record| record_json(db, record))
    else {
        return icons;
    };
    let Some(entries) = type_data
        .get("_RecordValue_")
        .unwrap_or(&type_data)
        .get("uiParams")
        .and_then(|u| u.get("typeData"))
        .and_then(|t| t.as_array())
    else {
        return icons;
    };
    for (index, name) in options.iter().enumerate() {
        if let Some(icon) = entries
            .get(index)
            .and_then(|entry| entry.get("typeIcon"))
            .and_then(|v| v.as_str())
            .filter(|icon| !icon.is_empty())
        {
            icons.insert((*name).to_string(), icon.to_string());
        }
    }
    icons
}

/// Build the power-screen binding-path overrides from the vehicle's pools and
/// its top-level fitted item records.
pub(crate) fn derive_power_paths(
    pools: &[PowerPool],
    fitted_items: &[Json],
    pool_defaults: &PoolDefaults,
    pool_icons: &HashMap<String, String>,
) -> HashMap<String, UiValue> {
    // Per pool: (total pips, overheat temperature of its consumers).
    let mut sized: Vec<(&PowerPool, u32, Option<f64>)> = pools
        .iter()
        .map(|pool| {
            if let Some(fixed) = pool.fixed_size {
                return (pool, fixed, None);
            }
            let mut units = 0u32;
            let mut overheat: Option<f64> = None;
            for item in fitted_items {
                if item_attach_type(item).as_deref() != Some(pool.item_type.as_str()) {
                    continue;
                }
                units += item_power_units(item);
                overheat = overheat.or_else(|| item_overheat_temperature(item));
            }
            (pool, units, overheat)
        })
        .collect();
    sized.sort_by_key(|(pool, _, _)| pool_display_rank(&pool.item_type));

    let mut paths: HashMap<String, UiValue> = HashMap::new();
    paths.insert("piplist".into(), UiValue::Int(sized.len() as i64));
    // Battery card: counts of fitted Battery items. At rest every fitted
    // battery is charged, so remaining == total (the Clipper fits none and
    // the in-game card reads "0 / 0").
    let battery_count = fitted_items
        .iter()
        .filter(|item| item_attach_type(item).as_deref() == Some("Battery"))
        .count() as i64;
    paths.insert("batteryremaining".into(), UiValue::Int(battery_count));
    paths.insert("batterytotal".into(), UiValue::Int(battery_count));
    // Emissions header (clone namespaces IR=[0000], EM=[0001], CS=[0002]).
    // TODO derive from the vehicle's signature components; the live formula
    // (engine SignatureSystem) is not yet decoded, so these are the
    // reference-verified at-rest Clipper values (Screen_Left_Lower_RTT).
    for (i, emitted, ambient) in
        [(0u32, 3500.0, 294.1), (1, 14900.0, 0.0), (2, 18600.0, 0.0)]
    {
        let base = format!("vehicle/signaturesystem/signatures/[{i:04}]");
        paths.insert(format!("{base}/emitted"), UiValue::Float(emitted));
        paths.insert(format!("{base}/ambient"), UiValue::Float(ambient));
    }
    let max_total = sized.iter().map(|(_, total, _)| *total).max().unwrap_or(0);
    paths.insert(
        "resourcenetworkui/powermanagement/pipsLengthMax".into(),
        UiValue::Int(max_total as i64),
    );

    for (i, (pool, total, overheat)) in sized.iter().enumerate() {
        let base = format!("piplist/[{i:04}]");
        let icon = pool_icons.get(&pool.item_type).cloned().unwrap_or_default();
        paths.insert(format!("{base}/itemicon"), UiValue::Str(icon));
        paths.insert(format!("{base}/piplinkamount"), UiValue::Int(1));
        paths.insert(format!("{base}/ispoweredoff"), UiValue::Bool(false));
        paths.insert(format!("{base}/piplist"), UiValue::Int(*total as i64));

        let in_use = (f64::from(*total) * pool_default_fraction(&pool.item_type, pool_defaults))
            .round() as u32;
        if in_use > 0 {
            paths.insert(
                format!("{base}/piplist/selectedindex"),
                UiValue::Int(i64::from(in_use) - 1),
            );
        }
        for j in 0..*total {
            let pip = format!("{base}/piplist/[{j:04}]");
            paths.insert(format!("{pip}/pipstate"), UiValue::Int(i64::from(j < in_use)));
            paths.insert(format!("{pip}/pipamount"), UiValue::Float(1.0));
            paths.insert(format!("{pip}/piplinkamount"), UiValue::Int(1));
        }

        // Heat gauge: only pools with a fitted consumer carrying a
        // temperature model show it (the reference hides the weapons gauge).
        let temp = format!("{base}/tempindicator");
        if pool.fixed_size.is_none() && *total > 0 {
            let overheat = overheat.unwrap_or(OVERHEAT_TEMP_K);
            paths.insert(format!("{temp}/currenttemp"), UiValue::Float(AMBIENT_TEMP_K));
            paths.insert(format!("{temp}/mintemp"), UiValue::Float(MIN_TEMP_K));
            paths.insert(format!("{temp}/overheattemp"), UiValue::Float(overheat));
            paths.insert(
                format!("{temp}/maxtemp"),
                UiValue::Float(overheat + OVERHEAT_HEADROOM_K),
            );
            paths.insert(format!("{temp}/tempdirection"), UiValue::Int(0));
        } else {
            paths.insert(format!("{temp}/maxtemp"), UiValue::Float(0.0));
        }
    }
    paths
}

#[cfg(test)]
mod tests;
