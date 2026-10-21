//! Tests for the per-ship power-screen value derivation: Clipper-shaped
//! fixtures assert pool parsing, consumption summing, in-use rounding,
//! display ordering, and the emitted binding paths match the
//! reference-verified at-rest model (4/6/4 pips, 2/3/2 in use, 7 pools).

use super::*;

fn clipper_pool_icons() -> HashMap<String, String> {
    [
        ("WeaponGun", "UI/Textures/Vector/General/CommonIcons/icon_common_weapon_gun.svg"),
        ("FlightController", "UI/Textures/Vector/Ships/ProfessionScreens/Engineering/Engineering_Icon_ItemThrusters.svg"),
        ("Shield", "UI/Textures/Vector/General/CommonIcons/icon_common_generator_shield.svg"),
        ("TractorBeam", "UI/Textures/Vector/General/CommonIcons/icon_common_tractor beam.svg"),
        ("TowingBeam", "UI/Textures/Vector/General/CommonIcons/icon_common_tractor beam.svg"),
        ("WeaponMining", "UI/Textures/Vector/General/CommonIcons/icon_common_mining.svg"),
        ("SalvageHead", "UI/Textures/Vector/General/CommonIcons/icon_common_salvage.svg"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}


fn clipper_pools_json() -> Json {
    serde_json::json!({
        "_RecordValue_": {
            "Components": [
                {
                    "_Type_": "SItemPortContainerComponentParams",
                    "resourceNetworkPowerPools": {
                        "itemPools": [
                            {"_Type_": "FixedPowerPool", "itemType": "WeaponGun", "poolSize": 4},
                            {"_Type_": "DynamicPowerPool", "itemType": "Shield", "maxItemCount": 2},
                            {"_Type_": "DynamicPowerPool", "itemType": "FlightController", "maxItemCount": -1},
                            {"_Type_": "DynamicPowerPool", "itemType": "TractorBeam", "maxItemCount": -1},
                            {"_Type_": "DynamicPowerPool", "itemType": "TowingBeam", "maxItemCount": -1},
                            {"_Type_": "DynamicPowerPool", "itemType": "WeaponMining", "maxItemCount": -1},
                            {"_Type_": "DynamicPowerPool", "itemType": "SalvageHead", "maxItemCount": -1},
                            null
                        ]
                    }
                }
            ]
        }
    })
}

fn item_json(attach_type: &str, power_units: u64, overheat: Option<f64>) -> Json {
    let mut components = vec![
        serde_json::json!({
            "_Type_": "SAttachableComponentParams",
            "AttachDef": {"Type": attach_type}
        }),
        serde_json::json!({
            "_Type_": "ItemResourceComponentParams",
            "states": [
                {
                    "name": "Online",
                    "deltas": [
                        {
                            "consumption": {
                                "resource": "Power",
                                "resourceAmountPerSecond": {
                                    "_Type_": "SPowerSegmentResourceUnit",
                                    "units": power_units
                                }
                            }
                        },
                        {
                            // Coolant consumption must not count as power.
                            "consumption": {
                                "resource": "Coolant",
                                "resourceAmountPerSecond": {
                                    "_Type_": "SStandardResourceUnit",
                                    "standardResourceUnits": 5.0
                                }
                            }
                        }
                    ]
                }
            ]
        }),
    ];
    if let Some(overheat) = overheat {
        components.push(serde_json::json!({
            "_Type_": "SEntityPhysicsControllerParams",
            "PhysType": {
                "temperature": {
                    "itemResourceParams": {"overheatTemperature": overheat}
                }
            }
        }));
    }
    serde_json::json!({"_RecordValue_": {"Components": components}})
}

fn clipper_fitted_items() -> Vec<Json> {
    vec![
        item_json("Shield", 2, Some(372.0)),
        item_json("Shield", 2, Some(372.0)),
        item_json("FlightController", 6, None),
        // A power plant GENERATES power; its record carries no power
        // consumption delta and must not contribute pips.
        item_json("PowerPlant", 0, Some(372.0)),
    ]
}

#[test]
fn vehicle_pools_parse_with_fixed_sizes() {
    let pools = power_pools_from_vehicle(&clipper_pools_json());
    assert_eq!(pools.len(), 7, "null pool entries are skipped");
    assert_eq!(pools[0], PowerPool { item_type: "WeaponGun".into(), fixed_size: Some(4) });
    assert_eq!(pools[1], PowerPool { item_type: "Shield".into(), fixed_size: None });
}

#[test]
fn item_power_units_sum_only_power_segment_units() {
    let item = item_json("Shield", 2, None);
    assert_eq!(item_power_units(&item), 2);
    assert_eq!(item_attach_type(&item).as_deref(), Some("Shield"));
}

#[test]
fn derived_clipper_paths_match_reference_model() {
    let pools = power_pools_from_vehicle(&clipper_pools_json());
    let items = clipper_fitted_items();
    let paths = derive_power_paths(&pools, &items, &PoolDefaults::default(), &clipper_pool_icons());

    assert_eq!(paths.get("piplist"), Some(&UiValue::Int(7)));
    assert_eq!(
        paths.get("resourcenetworkui/powermanagement/pipsLengthMax"),
        Some(&UiValue::Int(6)),
        "largest stack = engines"
    );

    // Display order: weapons, engines (FlightController), shields, then rest.
    assert_eq!(
        paths.get("piplist/[0000]/itemicon"),
        Some(&UiValue::Str(
            "UI/Textures/Vector/General/CommonIcons/icon_common_weapon_gun.svg".into()
        ))
    );
    assert_eq!(
        paths.get("piplist/[0001]/itemicon"),
        Some(&UiValue::Str(
            "UI/Textures/Vector/Ships/ProfessionScreens/Engineering/Engineering_Icon_ItemThrusters.svg".into()
        ))
    );
    assert_eq!(
        paths.get("piplist/[0002]/itemicon"),
        Some(&UiValue::Str(
            "UI/Textures/Vector/General/CommonIcons/icon_common_generator_shield.svg".into()
        ))
    );

    // 4/6/4 totals, 2/3/2 in use (bright bottom-up), selected = highest lit.
    for (i, total, in_use) in [(0u32, 4i64, 2i64), (1, 6, 3), (2, 4, 2)] {
        assert_eq!(paths.get(&format!("piplist/[{i:04}]/piplist")), Some(&UiValue::Int(total)));
        assert_eq!(
            paths.get(&format!("piplist/[{i:04}]/piplist/selectedindex")),
            Some(&UiValue::Int(in_use - 1))
        );
        for j in 0..total {
            let expected = i64::from(j < in_use);
            assert_eq!(
                paths.get(&format!("piplist/[{i:04}]/piplist/[{j:04}]/pipstate")),
                Some(&UiValue::Int(expected)),
                "pool {i} pip {j}"
            );
        }
    }

    // Heat gauges: engines + shields (dynamic, with consumers, overheat from
    // the item where modelled); weapons (fixed) and empty pools hide theirs.
    assert_eq!(
        paths.get("piplist/[0001]/tempindicator/overheattemp"),
        Some(&UiValue::Float(372.0)),
        "engines fall back to the small-component overheat family"
    );
    assert_eq!(
        paths.get("piplist/[0002]/tempindicator/overheattemp"),
        Some(&UiValue::Float(372.0)),
        "shield overheat comes from the fitted item"
    );
    assert_eq!(
        paths.get("piplist/[0000]/tempindicator/maxtemp"),
        Some(&UiValue::Float(0.0)),
        "fixed weapons pool hides the gauge"
    );
    assert_eq!(
        paths.get("piplist/[0003]/tempindicator/maxtemp"),
        Some(&UiValue::Float(0.0)),
        "empty pools hide the gauge"
    );
    // Empty pools render an icon and no pips.
    assert_eq!(paths.get("piplist/[0003]/piplist"), Some(&UiValue::Int(0)));
    assert!(paths.get("piplist/[0003]/piplist/selectedindex").is_none());
}

/// The battery card's `batteryremaining` / `batterytotal` engine variables
/// come from the fitted Battery items: none fitted (the Clipper) shows
/// "0 / 0" in the in-game reference; fitted batteries count as charged at
/// rest.
#[test]
fn battery_counts_come_from_fitted_items() {
    let pools = power_pools_from_vehicle(&clipper_pools_json());

    let no_batteries = clipper_fitted_items();
    let paths =
        derive_power_paths(&pools, &no_batteries, &PoolDefaults::default(), &clipper_pool_icons());
    assert_eq!(paths.get("batteryremaining"), Some(&UiValue::Int(0)));
    assert_eq!(paths.get("batterytotal"), Some(&UiValue::Int(0)));

    let mut with_batteries = clipper_fitted_items();
    with_batteries.push(item_json("Battery", 0, None));
    with_batteries.push(item_json("Battery", 0, None));
    let paths = derive_power_paths(
        &pools,
        &with_batteries,
        &PoolDefaults::default(),
        &clipper_pool_icons(),
    );
    assert_eq!(paths.get("batteryremaining"), Some(&UiValue::Int(2)));
    assert_eq!(paths.get("batterytotal"), Some(&UiValue::Int(2)));
}

/// The emissions header reads `vehicle/signaturesystem/signatures/[000i]`
/// (IR=0, EM=1, CS=2) emitted/ambient pairs via the clone namespaces.
#[test]
fn emission_signature_paths_are_emitted() {
    let pools = power_pools_from_vehicle(&clipper_pools_json());
    let items = clipper_fitted_items();
    let paths = derive_power_paths(&pools, &items, &PoolDefaults::default(), &clipper_pool_icons());

    assert_eq!(
        paths.get("vehicle/signaturesystem/signatures/[0000]/emitted"),
        Some(&UiValue::Float(3500.0)),
        "IR emitted"
    );
    assert_eq!(
        paths.get("vehicle/signaturesystem/signatures/[0000]/ambient"),
        Some(&UiValue::Float(294.1)),
        "IR ambient"
    );
    assert_eq!(
        paths.get("vehicle/signaturesystem/signatures/[0001]/emitted"),
        Some(&UiValue::Float(14900.0)),
        "EM emitted"
    );
    assert_eq!(
        paths.get("vehicle/signaturesystem/signatures/[0002]/emitted"),
        Some(&UiValue::Float(18600.0)),
        "CS emitted"
    );
    assert_eq!(
        paths.get("vehicle/signaturesystem/signatures/[0002]/ambient"),
        Some(&UiValue::Float(0.0)),
        "CS ambient"
    );
}

/// The gauge scale ceiling tracks the ITEM's real overheat threshold (the
/// Clipper's flight controller overheats at 450 K, its shields at 372 K), so
/// the heat-bar band ratios stay coherent (overheat never exceeds the scale).
#[test]
fn gauge_scale_tracks_item_overheat_threshold() {
    let pools = power_pools_from_vehicle(&clipper_pools_json());
    let items = vec![
        item_json("Shield", 2, Some(372.0)),
        item_json("FlightController", 6, Some(450.0)),
    ];
    let paths = derive_power_paths(&pools, &items, &PoolDefaults::default(), &clipper_pool_icons());

    assert_eq!(
        paths.get("piplist/[0001]/tempindicator/overheattemp"),
        Some(&UiValue::Float(450.0))
    );
    assert_eq!(
        paths.get("piplist/[0001]/tempindicator/maxtemp"),
        Some(&UiValue::Float(518.0)),
        "ceiling = overheat + reference-calibrated headroom"
    );
    assert_eq!(
        paths.get("piplist/[0002]/tempindicator/maxtemp"),
        Some(&UiValue::Float(440.0)),
        "shield scale matches the reference-calibrated 372+68"
    );
}

#[test]
fn compass_ticks_derive_from_tape_at_rest_heading() {
    // Default HUD compassTape: 90° window, labelled every 20°, 4 sub-ticks
    // (minor every 5°). At the neutral 0° heading the visible window is
    // [-45°, +45°] → 19 ticks at 5° spacing.
    let tape = CompassTape { range_deg: 90.0, main_tick_increment_deg: 20.0, sub_ticks: 4 };
    let paths = derive_compass_ticks(&tape, 0.0);
    let base = "FlightController/Compass/Ticks";

    assert_eq!(paths.get(base), Some(&UiValue::Int(19)), "count = window/minor + 1");

    // Entry 0 = leftmost tick at -45° → anchor 0.0, value 315, minor.
    assert_eq!(paths.get(&format!("{base}/[0000]/anchor")), Some(&UiValue::Float(0.0)));
    assert_eq!(paths.get(&format!("{base}/[0000]/value")), Some(&UiValue::Int(315)));
    assert_eq!(paths.get(&format!("{base}/[0000]/maintick")), Some(&UiValue::Bool(false)));

    // Centre entry (index 9) = 0° → anchor 0.5, value 0, labelled major.
    assert_eq!(paths.get(&format!("{base}/[0009]/anchor")), Some(&UiValue::Float(0.5)));
    assert_eq!(paths.get(&format!("{base}/[0009]/value")), Some(&UiValue::Int(0)));
    assert_eq!(paths.get(&format!("{base}/[0009]/maintick")), Some(&UiValue::Bool(true)));

    // Rightmost entry (index 18) = +45° → anchor 1.0, value 45, minor.
    assert_eq!(paths.get(&format!("{base}/[0018]/anchor")), Some(&UiValue::Float(1.0)));
    assert_eq!(paths.get(&format!("{base}/[0018]/value")), Some(&UiValue::Int(45)));

    // A negative-wrap major: -40° → value 320, labelled.
    assert_eq!(paths.get(&format!("{base}/[0001]/value")), Some(&UiValue::Int(320)));
    assert_eq!(paths.get(&format!("{base}/[0001]/maintick")), Some(&UiValue::Bool(true)));
    // A minor between majors: index 2 = -35° → value 325, minor.
    assert_eq!(paths.get(&format!("{base}/[0002]/value")), Some(&UiValue::Int(325)));
    assert_eq!(paths.get(&format!("{base}/[0002]/maintick")), Some(&UiValue::Bool(false)));
}

#[test]
fn compass_ticks_empty_for_degenerate_tape() {
    let tape = CompassTape { range_deg: 0.0, main_tick_increment_deg: 20.0, sub_ticks: 4 };
    assert!(derive_compass_ticks(&tape, 0.0).is_empty());
    let tape = CompassTape { range_deg: 90.0, main_tick_increment_deg: 20.0, sub_ticks: 0 };
    assert!(derive_compass_ticks(&tape, 0.0).is_empty());
}

fn ammo_container_item(initial: i64) -> Json {
    serde_json::json!({
        "_RecordValue_": {
            "Components": [
                {
                    "_Type_": "SAmmoContainerComponentParams",
                    "ammoContainerType": "Primary",
                    "initialAmmoCount": initial,
                    "maxAmmoCount": initial
                }
            ]
        }
    })
}

#[test]
fn countermeasure_initial_ammo_reads_container() {
    assert_eq!(countermeasure_initial_ammo(&ammo_container_item(7)), Some(7));
    // An item with no ammo container yields nothing.
    let no_container = serde_json::json!({"_RecordValue_": {"Components": []}});
    assert_eq!(countermeasure_initial_ammo(&no_container), None);
}

#[test]
fn countermeasure_paths_one_entry_per_launcher_in_loadout_order() {
    // Two launchers (loadout order); the ColumnReverse list lays index 0 at the
    // bottom panel and index 1 at the top.
    let paths = derive_countermeasure_paths(&[48, 5]);
    let base = "WeaponController/Countermeasures/Launchers";

    assert_eq!(paths.get(base), Some(&UiValue::Int(2)), "one entry per launcher");
    assert_eq!(paths.get(&format!("{base}/[0000]/AmmoCount")), Some(&UiValue::Int(48)));
    assert_eq!(paths.get(&format!("{base}/[0001]/AmmoCount")), Some(&UiValue::Int(5)));
    // The hold-to-fire overlay stays hidden at rest.
    assert_eq!(paths.get(&format!("{base}/[0000]/IsFiring")), Some(&UiValue::Bool(false)));
    assert_eq!(paths.get(&format!("{base}/[0001]/IsFiring")), Some(&UiValue::Bool(false)));
    assert_eq!(paths.get(&format!("{base}/[0000]/CurrentBurstSize")), Some(&UiValue::Int(0)));
    assert_eq!(
        paths.get(&format!("{base}/[0001]/BurstSizeHoldRatio")),
        Some(&UiValue::Float(0.0))
    );
}

#[test]
fn countermeasure_display_count_single_is_raw_burst_is_batches() {
    // SINGLE-fire launchers (noise) show the raw at-rest ammo.
    assert_eq!(countermeasure_display_count(5, false), 5);
    assert_eq!(countermeasure_display_count(48, false), 48);
    assert_eq!(countermeasure_display_count(0, false), 0);
    // BURST-fire launchers (decoy) show batches remaining = ceil(ammo / 12).
    // The Clipper's 48-flare BEHR decoy reads "4" in-game (reference-derived).
    assert_eq!(countermeasure_display_count(48, true), 4);
    assert_eq!(countermeasure_display_count(12, true), 1);
    assert_eq!(countermeasure_display_count(13, true), 2); // partial 2nd batch ceils up
    assert_eq!(countermeasure_display_count(0, true), 0); // empty → none left
    // Capital-grade decoys (ammo not a multiple of the charge) ceil to the next batch.
    assert_eq!(countermeasure_display_count(25, true), 3);
    assert_eq!(countermeasure_display_count(30, true), 3);
}

#[test]
fn countermeasure_is_burst_reads_fire_action_type() {
    let with_action = |ty: &str| {
        serde_json::json!({"_RecordValue_": {"Components": [
            {"_Type_": "SCItemWeaponComponentParams",
             "fireActions": [{"_Type_": ty, "name": "x"}]}]}})
    };
    assert!(countermeasure_is_burst(&with_action("SWeaponActionFireBurstParams")));
    assert!(!countermeasure_is_burst(&with_action("SWeaponActionFireSingleParams")));
    // No weapon component → not a burst launcher.
    let no_weapon = serde_json::json!({"_RecordValue_": {"Components": []}});
    assert!(!countermeasure_is_burst(&no_weapon));
}

#[test]
fn countermeasure_paths_empty_without_launchers() {
    assert!(derive_countermeasure_paths(&[]).is_empty());
}
