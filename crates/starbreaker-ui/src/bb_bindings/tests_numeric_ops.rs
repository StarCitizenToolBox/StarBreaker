//! Tests for the numeric binding ops used by the power-screen pip sizing
//! chains (`gen_mc_s_powerlistitem`): `NumberFromBoolean` branch selection,
//! `BooleanFromNumber` comparisons, `NumberRound` (decimal places),
//! `NumberFromIntegerSwitch`, and `NumberArithmatic` with an absent LHS
//! falling back to `amount` (the engine's `Div(amount=1, inputB=x)` = `1/x`).

use crate::canvas::Value;
use crate::defaults::DefaultValueRegistry;

use super::BindingResolver;

fn resolver_for(ops: serde_json::Value) -> BindingResolver {
    let ops_vec: Vec<serde_json::Value> = ops.as_array().expect("ops array").clone();
    BindingResolver::from_operations(&ops_vec)
}

/// `NumberFromBoolean` selects the `inputTrue`/`inputFalse` branch POINTER
/// when wired, with the `isTrue`/`isFalse` literals as fallbacks; the boolean
/// input compares a number against the authored literal (`Greater 15`, the
/// pip width/height fallback threshold).
#[test]
fn number_from_boolean_selects_branch_by_number_comparison() {
    let ops = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsNumberField",
         "widget": "_PointsTo_:ptr:1", "field": "SizeX", "input": "_PointsTo_:ptr:10"},
        {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberFromBoolean",
         "isTrue": 0.5, "isFalse": 1.0, "inputTrue": null, "inputFalse": null,
         "input": "_PointsTo_:ptr:11"},
        {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsBooleanFromNumber",
         "type": "Greater", "number": 15.0, "input": "_PointsTo_:ptr:12", "inputB": null},
        {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsNumberVariable",
         "path": [], "binding": "count", "inheritsNamespace": true}
    ]);
    let resolver = resolver_for(ops);

    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("count", Value::Float(3.0));
    assert_eq!(
        resolver.resolve_field_number(1, "SizeX", &defaults),
        Some(1.0),
        "3 > 15 is false: the isFalse literal"
    );

    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("count", Value::Float(16.0));
    assert_eq!(
        resolver.resolve_field_number(1, "SizeX", &defaults),
        Some(0.5),
        "16 > 15 is true: the isTrue literal"
    );
}

/// `NumberRound` rounds to `amount` DECIMAL PLACES, and a `NumberArithmatic`
/// `Div` with no `input` uses `amount` as its left operand: the pip slot
/// height `round3(1 / MaxPipList)`.
#[test]
fn number_round_and_div_amount_lhs() {
    let ops = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsNumberField",
         "widget": "_PointsTo_:ptr:1", "field": "SizeY", "input": "_PointsTo_:ptr:10"},
        {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberRound",
         "amount": 3, "input": "_PointsTo_:ptr:11"},
        {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
         "type": "Div", "amount": 1.0, "input": null, "inputB": "_PointsTo_:ptr:12"},
        {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsNumberVariable",
         "path": [], "binding": "max", "inheritsNamespace": true}
    ]);
    let resolver = resolver_for(ops);
    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("max", Value::Float(6.0));
    let value = resolver
        .resolve_field_number(1, "SizeY", &defaults)
        .expect("resolves");
    assert!((value - 0.167).abs() < 1e-9, "round3(1/6) = 0.167, got {value}");
}

/// `NumberFromIntegerSwitch` maps integer cases to numbers with a default:
/// `switch(pipState; 5 → 1.0; default 0)` forces the PreAssigned pip's fill.
#[test]
fn number_from_integer_switch_maps_cases() {
    let ops = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsNumberField",
         "widget": "_PointsTo_:ptr:1", "field": "SizeX", "input": "_PointsTo_:ptr:10"},
        {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
         "type": "Max", "amount": 1.0, "input": "_PointsTo_:ptr:11", "inputB": "_PointsTo_:ptr:12"},
        {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsNumberVariable",
         "path": [], "binding": "pipamount", "inheritsNamespace": true},
        {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsNumberFromIntegerSwitch",
         "defaultValue": 0.0, "defaultOverride": null, "input": "_PointsTo_:ptr:13",
         "values": [{"_Type_": "BuildingBlocks_IntegerNumberPair", "first": 5, "second": 1.0}]},
        {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
         "path": [], "binding": "pipstate", "inheritsNamespace": true}
    ]);
    let resolver = resolver_for(ops);

    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("pipamount", Value::Float(0.3));
    defaults.insert_path("pipstate", Value::Int(5));
    assert_eq!(
        resolver.resolve_field_number(1, "SizeX", &defaults),
        Some(1.0),
        "state 5 (PreAssigned) forces full fill via Max(0.3, 1.0)"
    );

    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("pipamount", Value::Float(0.3));
    defaults.insert_path("pipstate", Value::Int(1));
    assert_eq!(
        resolver.resolve_field_number(1, "SizeX", &defaults),
        Some(0.3),
        "other states fall to the switch default 0: Max(0.3, 0.0)"
    );
}

/// The full authored pip-slot SizeY chain: `pipLinkAmount > 1 ? slot×links :
/// slot` where `slot = MaxPipList > 15 ? PipHeight : round3(1/MaxPipList)` —
/// with the engines column's MaxPipList = 6 every slot is 1/6 of the column.
#[test]
fn pip_slot_size_y_chain_resolves_one_sixth() {
    let ops = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsNumberField",
         "widget": "_PointsTo_:ptr:2", "field": "SizeY", "input": "_PointsTo_:ptr:53"},
        {"_Pointer_": "ptr:53", "_Type_": "BuildingBlocks_BindingsNumberFromBoolean",
         "isTrue": 0.0, "isFalse": 1.0,
         "inputTrue": "_PointsTo_:ptr:75", "inputFalse": "_PointsTo_:ptr:76",
         "input": "_PointsTo_:ptr:77"},
        {"_Pointer_": "ptr:77", "_Type_": "BuildingBlocks_BindingsBooleanFromInteger",
         "type": "Greater", "inputL": "_PointsTo_:ptr:78", "inputR": null, "value": 1},
        {"_Pointer_": "ptr:78", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
         "path": [], "binding": "piplinkamount", "inheritsNamespace": true},
        {"_Pointer_": "ptr:75", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
         "type": "Mul", "amount": 1.0, "input": "_PointsTo_:ptr:76", "inputB": "_PointsTo_:ptr:79"},
        {"_Pointer_": "ptr:79", "_Type_": "BuildingBlocks_BindingsNumberFromInteger",
         "asSeconds": false, "input": "_PointsTo_:ptr:78"},
        {"_Pointer_": "ptr:76", "_Type_": "BuildingBlocks_BindingsNumberFromBoolean",
         "isTrue": 0.0, "isFalse": 1.0,
         "inputTrue": "_PointsTo_:ptr:56", "inputFalse": "_PointsTo_:ptr:57",
         "input": "_PointsTo_:ptr:58"},
        {"_Pointer_": "ptr:58", "_Type_": "BuildingBlocks_BindingsBooleanFromNumber",
         "type": "Greater", "number": 15.0, "input": "_PointsTo_:ptr:55", "inputB": null},
        {"_Pointer_": "ptr:55", "_Type_": "BuildingBlocks_BindingsNumberFromInteger",
         "asSeconds": false, "input": "_PointsTo_:ptr:54"},
        {"_Pointer_": "ptr:54", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
         "path": [], "binding": "maxpiplist", "inheritsNamespace": true},
        {"_Pointer_": "ptr:56", "_Type_": "BuildingBlocks_BindingsNumberComponentParameter",
         "name": "Pip height", "parameter": "ParamInput1", "defaultValue": 0.06},
        {"_Pointer_": "ptr:57", "_Type_": "BuildingBlocks_BindingsNumberRound",
         "amount": 3, "input": "_PointsTo_:ptr:82"},
        {"_Pointer_": "ptr:82", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
         "type": "Div", "amount": 1.0, "input": null, "inputB": "_PointsTo_:ptr:62"},
        {"_Pointer_": "ptr:62", "_Type_": "BuildingBlocks_BindingsNumberFromInteger",
         "asSeconds": false, "input": "_PointsTo_:ptr:54"}
    ]);
    let resolver = resolver_for(ops);
    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("maxpiplist", Value::Int(6));
    defaults.insert_path("piplinkamount", Value::Int(1));
    let value = resolver
        .resolve_field_number(2, "SizeY", &defaults)
        .expect("resolves");
    assert!((value - 0.167).abs() < 1e-9, "slot height = round3(1/6), got {value}");
}

/// `IntegerFromNumber` bridges the number domain into the integer/localized
/// chain: the OUTPUT card's "2" / "/ 16" run
/// `LocalizedFromInteger(IntegerFromNumber(NumberVariable))`.
#[test]
fn localized_from_integer_resolves_through_integer_from_number() {
    let ops = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsLocalizedField",
         "widget": "_PointsTo_:ptr:1", "field": "ParamInput0", "input": "_PointsTo_:ptr:10"},
        {"_Pointer_": "ptr:10", "_Type_": "BindingsOperations_LocalizationCombine",
         "inputL": null, "inputR": "_PointsTo_:ptr:11", "value": "@LOC_FORWARDSLASH",
         "withSpace": true},
        {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsLocalizedFromInteger",
         "defaultNZeros": 0, "nZeros": null, "withSeparators": true,
         "input": "_PointsTo_:ptr:12"},
        {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsIntegerFromNumber",
         "input": "_PointsTo_:ptr:13"},
        {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsNumberVariable",
         "path": [], "binding": "totalpossiblepower", "inheritsNamespace": true}
    ]);
    let resolver = resolver_for(ops);

    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_localization("loc_forwardslash", "/".to_string());
    defaults.insert_path("totalpossiblepower", Value::Float(16.0));
    assert_eq!(
        resolver.resolve_field_text(1, "ParamInput0", &defaults).as_deref(),
        Some("/ 16"),
        "the OUTPUT total renders the slash-prefixed integer"
    );
}

/// Bound `AnchorY` number fields apply to node anchors (the heat gauge's
/// `CurrentVelocityMarker` binds `AnchorY = 1 - currentTemp/maxTemp` so the
/// marker rides the temperature; the authored anchor 0 is an editor rest
/// pose). Unresolvable chains keep the authored anchor.
#[test]
fn bound_anchor_fields_apply_to_node_anchor() {
    let canvas = serde_json::json!({
        "_RecordValue_": {
            "_Type_": "BuildingBlocks_Canvas",
            "size": {"x": 100.0, "y": 100.0},
            "coordinateMethod": "useRaw",
            "scene": [
                {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                 "name": "marker", "isActive": true,
                 "anchor": {"x": 1.0, "y": 0.0, "z": 0.0},
                 "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                            "height": {"value": 13.0, "behavior": "Fixed"}}}
            ],
            "operations": [
                {"_Type_": "BuildingBlocks_BindingsNumberField",
                 "widget": "_PointsTo_:ptr:1", "field": "AnchorY",
                 "input": "_PointsTo_:ptr:10"},
                {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
                 "type": "Sub", "amount": 1.0, "input": null, "inputB": "_PointsTo_:ptr:11"},
                {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
                 "type": "Div", "amount": 1.0, "input": "_PointsTo_:ptr:12", "inputB": "_PointsTo_:ptr:13"},
                {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                 "path": [], "binding": "tempindicator/currenttemp", "inheritsNamespace": true},
                {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                 "path": [], "binding": "tempindicator/maxtemp", "inheritsNamespace": true}
            ]
        }
    });
    let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("fixture parses");

    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("tempindicator/currenttemp", Value::Float(290.0));
    defaults.insert_path("tempindicator/maxtemp", Value::Float(518.0));
    super::resolve_geometry_fields_into_scene(&mut scene, &defaults);
    let anchor_y = scene.nodes[&1].anchor.y;
    assert!(
        (anchor_y - (1.0 - 290.0 / 518.0)).abs() < 1e-4,
        "bound AnchorY applies: got {anchor_y}"
    );
    assert_eq!(scene.nodes[&1].anchor.x, 1.0, "unbound axis untouched");
}

/// A bound anchor whose chain cannot resolve at all (a bare unbound engine
/// variable) keeps the authored anchor.
#[test]
fn unresolvable_bound_anchor_keeps_authored_value() {
    let canvas = serde_json::json!({
        "_RecordValue_": {
            "_Type_": "BuildingBlocks_Canvas",
            "size": {"x": 100.0, "y": 100.0},
            "coordinateMethod": "useRaw",
            "scene": [
                {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                 "name": "marker", "isActive": true,
                 "anchor": {"x": 0.5, "y": 0.25, "z": 0.0},
                 "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                            "height": {"value": 13.0, "behavior": "Fixed"}}}
            ],
            "operations": [
                {"_Type_": "BuildingBlocks_BindingsNumberField",
                 "widget": "_PointsTo_:ptr:1", "field": "AnchorY",
                 "input": "_PointsTo_:ptr:10"},
                {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                 "path": [], "binding": "some/unbound/value", "inheritsNamespace": true}
            ]
        }
    });
    let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("fixture parses");
    let empty = DefaultValueRegistry::default();
    super::resolve_geometry_fields_into_scene(&mut scene, &empty);
    assert_eq!(scene.nodes[&1].anchor.y, 0.25, "authored anchor survives");
}

/// `LocalizedFromNumber` formats with `nPlaces` decimals and
/// `LocalizedSIUnitFromNumber` adds the SI magnitude prefix — the emissions
/// header's "3.5K" emitted / "294.1" ambient pair.
#[test]
fn localized_number_and_si_unit_formatting() {
    let ops = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsLocalizedField",
         "widget": "_PointsTo_:ptr:1", "field": "ParamInput0", "input": "_PointsTo_:ptr:10"},
        {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsLocalizedSIUnitFromNumber",
         "nPlaces": 1, "nPlacesBinding": null, "withSeparators": false,
         "unitSuffix": "None", "unitSuffixBinding": null, "forcedSIPrefix": "INVALID",
         "input": "_PointsTo_:ptr:12"},
        {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsNumberVariable",
         "path": [], "binding": "emitted", "inheritsNamespace": true},
        {"_Type_": "BuildingBlocks_BindingsLocalizedField",
         "widget": "_PointsTo_:ptr:2", "field": "ParamInput0", "input": "_PointsTo_:ptr:11"},
        {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsLocalizedFromNumber",
         "nZeros": 0, "nPlaces": 1, "trailingZeros": true, "withSeparators": false,
         "input": "_PointsTo_:ptr:13"},
        {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsNumberVariable",
         "path": [], "binding": "ambient", "inheritsNamespace": true}
    ]);
    let resolver = resolver_for(ops);
    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("emitted", Value::Float(3500.0));
    defaults.insert_path("ambient", Value::Float(294.1));

    assert_eq!(
        resolver.resolve_field_text(1, "ParamInput0", &defaults).as_deref(),
        Some("3.5K"),
        "SI prefix at one decimal place"
    );
    assert_eq!(
        resolver.resolve_field_text(2, "ParamInput0", &defaults).as_deref(),
        Some("294.1"),
        "plain number at one decimal place"
    );

    // Below the SI threshold the SI op formats the plain number.
    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("emitted", Value::Float(294.1));
    assert_eq!(
        resolver.resolve_field_text(1, "ParamInput0", &defaults).as_deref(),
        Some("294.1")
    );
}

/// A boolean `ComponentParameter` with no parent wiring takes a registry
/// value matching its NAME before the editor `defaultValue` — `iscast`
/// defaults TRUE in data (the editor pose) but the engine wires it FALSE for
/// screen render targets; the at-rest registry carries that engine value.
#[test]
fn boolean_param_takes_registry_value_by_name_over_editor_default() {
    let ops = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsBooleanField",
         "widget": "_PointsTo_:ptr:1", "field": "IsActive", "input": "_PointsTo_:ptr:10"},
        {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
         "name": "iscast", "parameter": "ParamInput0", "defaultValue": true}
    ]);
    let resolver = resolver_for(ops);

    let empty = DefaultValueRegistry::default();
    assert_eq!(
        resolver.resolve_field_bool(1, "IsActive", &empty),
        Some(true),
        "no registry value: the authored editor default"
    );

    let mut defaults = DefaultValueRegistry::default();
    defaults.insert_path("iscast", Value::Bool(false));
    assert_eq!(
        resolver.resolve_field_bool(1, "IsActive", &defaults),
        Some(false),
        "registry at-rest value wins over the editor default"
    );
}

/// A widget size driven by a live engine `Variable` (the velocity ball's
/// `SizeY = |flightcontroller/linearvelocity/ratio/z| / 2`) is recognised as
/// engine-driven, so its at-rest `0` is a genuine collapse — distinct from the
/// power-pip's `1/MaxPipList` (a divide-by-zero of an unwired component
/// parameter) and a direct unwired `ComponentParameter` default, which are
/// half-resolved placeholders that must keep their authored size.
#[test]
fn engine_variable_size_source_distinguishes_genuine_zero_from_placeholder() {
    // velocity: Div(amount=2, input = NumberFromBoolean(inputFalse = velocity var)).
    let velocity = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsNumberField",
         "widget": "_PointsTo_:ptr:12", "field": "SizeY", "input": "_PointsTo_:ptr:13"},
        {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
         "type": "Div", "amount": 2.0, "input": "_PointsTo_:ptr:25", "inputB": null},
        {"_Pointer_": "ptr:25", "_Type_": "BuildingBlocks_BindingsNumberFromBoolean",
         "isTrue": 0.0, "isFalse": 1.0, "inputTrue": "_PointsTo_:ptr:17", "inputFalse": "_PointsTo_:ptr:14",
         "input": "_PointsTo_:ptr:15"},
        {"_Pointer_": "ptr:17", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
         "type": "Mul", "amount": -1.0, "input": "_PointsTo_:ptr:14", "inputB": null},
        {"_Pointer_": "ptr:15", "_Type_": "BuildingBlocks_BindingsBooleanFromNumber",
         "type": "Less", "number": 0.0, "input": "_PointsTo_:ptr:14", "inputB": null},
        {"_Pointer_": "ptr:14", "_Type_": "BuildingBlocks_BindingsNumberVariable",
         "path": [], "binding": "flightcontroller/linearvelocity/ratio/z", "inheritsNamespace": true}
    ]);
    let resolver = resolver_for(velocity);
    let empty = DefaultValueRegistry::default();
    assert_eq!(
        resolver.resolve_field_number(12, "SizeY", &empty),
        Some(0.0),
        "at rest |0|/2 = 0"
    );
    assert!(
        resolver.field_value_source_is_engine_variable(12, "SizeY", &empty),
        "the size flows from the velocity engine Variable: a genuine collapse"
    );

    // pip: Div(amount=1, input=null, inputB = unwired MaxPipList ComponentParameter) → 1/0.
    let pip = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsNumberField",
         "widget": "_PointsTo_:ptr:2", "field": "SizeY", "input": "_PointsTo_:ptr:82"},
        {"_Pointer_": "ptr:82", "_Type_": "BuildingBlocks_BindingsNumberArithmatic",
         "type": "Div", "amount": 1.0, "input": null, "inputB": "_PointsTo_:ptr:62"},
        {"_Pointer_": "ptr:62", "_Type_": "BuildingBlocks_BindingsNumberFromInteger",
         "asSeconds": false, "input": "_PointsTo_:ptr:61"},
        {"_Pointer_": "ptr:61", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
         "name": "Max pipList", "parameter": "ParamInput2", "defaultValue": 0}
    ]);
    let resolver = resolver_for(pip);
    assert!(
        !resolver.field_value_source_is_engine_variable(2, "SizeY", &empty),
        "1/MaxPipList is a divide-by-zero of an unwired parameter: a placeholder, not engine-driven"
    );

    // direct unwired component parameter size (widget-standard expansion icon).
    let param = serde_json::json!([
        {"_Type_": "BuildingBlocks_BindingsNumberField",
         "widget": "_PointsTo_:ptr:3", "field": "SizeX", "input": "_PointsTo_:ptr:90"},
        {"_Pointer_": "ptr:90", "_Type_": "BuildingBlocks_BindingsNumberComponentParameter",
         "name": "Icon size", "parameter": "ParamInput0", "defaultValue": 0.0}
    ]);
    let resolver = resolver_for(param);
    assert!(
        !resolver.field_value_source_is_engine_variable(3, "SizeX", &empty),
        "an unwired component parameter is not engine-driven"
    );
}
