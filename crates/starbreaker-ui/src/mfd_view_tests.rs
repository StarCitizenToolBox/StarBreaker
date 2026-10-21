//! Tests for [`crate::mfd_view`]: slot identification, bound-view injection,
//! host content-view sub-rect sizing, and the legacy by-canvas fallback.

use std::collections::HashSet;

use super::*;



/// An MFD content frame wired like `m_eng_mfdcontent`: two view slots whose
/// `CanvasReferenceRecord` is bound to `landscapecanvasguid` /
/// `portraitcanvasguid`, each gated by `useportraitview`. The authored
/// placeholders deliberately differ from the bound content to prove the
/// injection does not depend on a placeholder match.
fn mfd_content_frame() -> serde_json::Value {
    serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.M_Eng_MfdContent",
        "_RecordValue_": {
            "size": {"x": 800, "y": 600},
            "scene": [
                {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_content",
                 "isActive": true, "sizing": {"width": {"behavior": "Fixed", "value": 800.0}, "height": {"behavior": "Fixed", "value": 600.0}}},
                {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "canvas_LandscapeMFDView",
                 "parent": "_PointsTo_:ptr:1", "isActive": true, "layer": 5, "instantiated": true,
                 "sizing": {"width": {"behavior": "Percent", "value": 1.0}, "height": {"behavior": "Percent", "value": 1.0}},
                 "canvas": "file://./self_placeholder.json"},
                {"_Pointer_": "ptr:9", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "canvas_PortraitMFDView",
                 "parent": "_PointsTo_:ptr:1", "isActive": true, "layer": 5, "instantiated": true,
                 "sizing": {"width": {"behavior": "PercentOfY", "value": 0.7}, "height": {"behavior": "Percent", "value": 1.0}},
                 "canvas": "file://./target_placeholder.json"}
            ],
            "operations": [
                {"_Type_": "BuildingBlocks_BindingsStringField", "widget": "_PointsTo_:ptr:13",
                 "field": "CanvasReferenceRecord", "input": "_PointsTo_:ptr:14"},
                {"_Pointer_": "ptr:14", "_Type_": "BuildingBlocks_BindingsStringVariable",
                 "binding": "landscapecanvasguid", "inheritsNamespace": true},
                {"_Type_": "BuildingBlocks_BindingsStringField", "widget": "_PointsTo_:ptr:9",
                 "field": "CanvasReferenceRecord", "input": "_PointsTo_:ptr:16"},
                {"_Pointer_": "ptr:16", "_Type_": "BuildingBlocks_BindingsStringVariable",
                 "binding": "portraitcanvasguid", "inheritsNamespace": true},
                {"_Type_": "BuildingBlocks_BindingsBooleanField", "widget": "_PointsTo_:ptr:9",
                 "field": "Instantiated", "input": "_PointsTo_:ptr:27"},
                {"_Pointer_": "ptr:27", "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                 "binding": "useportraitview", "inheritsNamespace": true},
                {"_Type_": "BuildingBlocks_BindingsBooleanField", "widget": "_PointsTo_:ptr:13",
                 "field": "Instantiated", "input": "_PointsTo_:ptr:30"},
                {"_Pointer_": "ptr:30", "_Type_": "BuildingBlocks_BindingsBooleanInvert", "input": "_PointsTo_:ptr:27"}
            ]
        }
    })
}

fn node_id(scene: &BbScene, name: &str) -> BbNodeId {
    *scene.nodes.iter().find(|(_, n)| n.name == name).unwrap().0
}

#[test]
fn identifies_landscape_and_portrait_slots_from_canvas_bindings() {
    let scene = crate::bb_scene::parse_bb_canvas(&mfd_content_frame()).expect("parse");
    let slots = mfd_view_slots(&scene).expect("frame exposes the landscape/portrait wiring");
    assert_eq!(slots.landscape, node_id(&scene, "canvas_LandscapeMFDView"));
    assert_eq!(slots.portrait, node_id(&scene, "canvas_PortraitMFDView"));
}

#[test]
fn injects_bound_content_onto_landscape_slot_and_drops_portrait() {
    let mut scene = crate::bb_scene::parse_bb_canvas(&mfd_content_frame()).expect("parse");
    let landscape = node_id(&scene, "canvas_LandscapeMFDView");
    let portrait = node_id(&scene, "canvas_PortraitMFDView");

    let mut canvas_urls: Vec<CanvasUrl> = vec![
        (landscape, "file://./self_placeholder.json".to_string(), Vec::new()),
        (portrait, "file://./target_placeholder.json".to_string(), Vec::new()),
    ];
    // The portrait slot is the static-default active one; the landscape slot
    // is forced inactive — exactly the inverted state the old matcher left.
    let mut inst_false: HashSet<BbNodeId> = HashSet::from([landscape]);

    apply_bound_mfd_view(
        &mut scene,
        "BuildingBlocks_Canvas.MC_S_Scanning_Master",
        &mut canvas_urls,
        &mut inst_false,
    );

    // Landscape now active and following the bound (scanning) content.
    assert!(!inst_false.contains(&landscape), "landscape slot must be instantiated");
    assert!(inst_false.contains(&portrait), "portrait slot must be skipped");
    let landscape_url = &canvas_urls.iter().find(|(id, _, _)| *id == landscape).unwrap().1;
    assert_eq!(landscape_url, "BuildingBlocks_Canvas.MC_S_Scanning_Master");
    // Portrait's placeholder URL is untouched (it is skipped via inst_false).
    let portrait_url = &canvas_urls.iter().find(|(id, _, _)| *id == portrait).unwrap().1;
    assert_eq!(portrait_url, "file://./target_placeholder.json");
}

/// The GFx host places the content view in a stage sub-rect inset from the
/// 1280×720 stage (44px left/right/bottom, flush top). The landscape slot
/// must be sized to that fraction of the frame, x-centred and top-anchored,
/// so the content maps like the in-game capture (x-scale 0.93125 centred,
/// y-scale 676/720 top-anchored) instead of full-bleed.
#[test]
fn landscape_slot_is_sized_to_host_content_view_sub_rect() {
    let mut scene = crate::bb_scene::parse_bb_canvas(&mfd_content_frame()).expect("parse");
    let landscape = node_id(&scene, "canvas_LandscapeMFDView");

    apply_bound_mfd_view(
        &mut scene,
        "BuildingBlocks_Canvas.MC_S_Scanning_Master",
        &mut Vec::new(),
        &mut HashSet::new(),
    );

    let slot = scene.nodes.get(&landscape).unwrap();
    let crate::bb_scene::BbValue::Percent(w) = slot.sizing.width else {
        panic!("slot width must be Percent, got {:?}", slot.sizing.width);
    };
    let crate::bb_scene::BbValue::Percent(h) = slot.sizing.height else {
        panic!("slot height must be Percent, got {:?}", slot.sizing.height);
    };
    assert!((w - (1192.0 / 1280.0)).abs() < 1e-4, "width fraction, got {w}");
    assert!((h - (676.0 / 720.0)).abs() < 1e-4, "height fraction, got {h}");
    assert_eq!((slot.anchor.x, slot.anchor.y), (0.5, 0.0), "x-centred, top-anchored");
    assert_eq!((slot.pivot.x, slot.pivot.y), (0.5, 0.0));
}

/// The slot fractions follow the binding's HOST SWF stage size when the
/// pipeline provides one (read from the movie header, plan P5.4); the
/// (1280,720) constant is only the no-host fallback. The 44px inset stays a
/// stage-pixel quantity, so a synthetic 1000×500 stage yields 912/1000 and
/// 456/500.
#[test]
fn landscape_slot_fractions_follow_the_host_stage_size() {
    let mut scene = crate::bb_scene::parse_bb_canvas(&mfd_content_frame()).expect("parse");
    let landscape = node_id(&scene, "canvas_LandscapeMFDView");

    apply_bound_mfd_view_with_host_stage(
        &mut scene,
        "BuildingBlocks_Canvas.MC_S_Scanning_Master",
        &mut Vec::new(),
        &mut HashSet::new(),
        Some((1000.0, 500.0)),
    );

    let slot = scene.nodes.get(&landscape).unwrap();
    let crate::bb_scene::BbValue::Percent(w) = slot.sizing.width else {
        panic!("slot width must be Percent, got {:?}", slot.sizing.width);
    };
    let crate::bb_scene::BbValue::Percent(h) = slot.sizing.height else {
        panic!("slot height must be Percent, got {:?}", slot.sizing.height);
    };
    assert!((w - (912.0 / 1000.0)).abs() < 1e-4, "width fraction, got {w}");
    assert!((h - (456.0 / 500.0)).abs() < 1e-4, "height fraction, got {h}");
}

/// End-to-end through the resolver: the bound content (matching NEITHER
/// authored placeholder) must merge under the full-width landscape slot, and
/// neither placeholder's content nor the portrait slot may render. This is
/// the obs-3 regression — the bound target content was being squashed into
/// the narrow portrait slot.
#[test]
fn resolver_renders_bound_content_full_width_in_landscape_slot() {
    let frame = mfd_content_frame();
    let view = |name: &str, marker: &str| serde_json::json!({
        "_RecordName_": format!("BuildingBlocks_Canvas.{name}"),
        "_RecordValue_": {"size": {"x": 800, "y": 600}, "scene": [
            {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetTextField", "name": marker,
             "isActive": true, "text": "x",
             "sizing": {"width": {"behavior": "Fixed", "value": 100.0}, "height": {"behavior": "Fixed", "value": 20.0}}}
        ], "operations": []}
    });
    let self_view = view("Self_Placeholder", "marker_self");
    let target_view = view("Target_Placeholder", "marker_target");
    let scanning_view = view("MC_S_Scanning_Master", "marker_scanning");
    let fetch = move |path: &str| -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("self_placeholder") {
            Ok(self_view.clone())
        } else if p.contains("target_placeholder") {
            Ok(target_view.clone())
        } else if p.contains("scanning") {
            Ok(scanning_view.clone())
        } else {
            Err(format!("unknown canvas: {path}"))
        }
    };

    let scene = crate::bb_resolve::resolve_canvas_graph_with_loc_and_bound_view(
        &frame,
        Some("drak"),
        &fetch,
        None,
        Some("BuildingBlocks_Canvas.MC_S_Scanning_Master"),
    )
    .expect("resolve");

    let active_names: Vec<&str> = scene
        .nodes
        .values()
        .filter(|n| n.is_active)
        .map(|n| n.name.as_str())
        .collect();
    assert!(
        active_names.contains(&"marker_scanning"),
        "bound content must render in the landscape slot; got {active_names:?}"
    );
    assert!(
        !active_names.contains(&"marker_self"),
        "landscape placeholder content must be replaced; got {active_names:?}"
    );
    assert!(
        !active_names.contains(&"marker_target"),
        "portrait slot content must not render; got {active_names:?}"
    );

    // The bound content is parented under the full-width landscape slot.
    let scanning_id = node_id(&scene, "marker_scanning");
    let landscape_id = node_id(&scene, "canvas_LandscapeMFDView");
    assert!(
        ancestor_chain(&scene, scanning_id).contains(&landscape_id),
        "bound content must be a descendant of canvas_LandscapeMFDView"
    );
}

/// Walk `node`'s parent chain (inclusive) to its root.
fn ancestor_chain(scene: &BbScene, node: BbNodeId) -> Vec<BbNodeId> {
    let mut chain = vec![node];
    let mut cur = node;
    while let Some(parent) = scene.nodes.get(&cur).and_then(|n| n.parent) {
        chain.push(parent);
        cur = parent;
    }
    chain
}

/// A frame embedding two mutually-exclusive content views by authored
/// `canvas:` placeholder (no `landscapecanvasguid` wiring) still selects the
/// bound view via the legacy by-canvas matcher.
fn legacy_frame_scene() -> serde_json::Value {
    serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.LegacyFrame",
        "_RecordValue_": {
            "size": {"x": 800, "y": 600},
            "scene": [
                {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_content",
                 "isActive": true, "sizing": {"width": {"behavior": "Fixed", "value": 800.0}, "height": {"behavior": "Fixed", "value": 600.0}}},
                {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "canvas_A",
                 "parent": "_PointsTo_:ptr:1", "isActive": true, "layer": 5,
                 "canvas": "file://./screens/target/mc_s_target_master.json"},
                {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "canvas_B",
                 "parent": "_PointsTo_:ptr:1", "isActive": true, "layer": 5,
                 "canvas": "file://./screens/self/mc_s_self_master.json"},
                {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "canvas_Header / Footer",
                 "parent": "_PointsTo_:ptr:1", "isActive": true, "layer": 10,
                 "canvas": "file://./header_types/gen_mc_s_header.json"}
            ],
            "operations": []
        }
    })
}

#[test]
fn legacy_forces_bound_view_on_and_sibling_off_keeping_footer() {
    let mut scene = crate::bb_scene::parse_bb_canvas(&legacy_frame_scene()).expect("parse");
    let a = node_id(&scene, "canvas_A");
    let b = node_id(&scene, "canvas_B");
    let footer = node_id(&scene, "canvas_Header / Footer");

    let mut inst_false: HashSet<BbNodeId> = HashSet::from([a]);
    // No landscapecanvasguid wiring → falls through to the by-canvas matcher.
    apply_bound_mfd_view(
        &mut scene,
        "BuildingBlocks_Canvas.MC_S_Target_Master",
        &mut Vec::new(),
        &mut inst_false,
    );

    assert!(!inst_false.contains(&a), "bound target slot must be instantiated");
    assert!(inst_false.contains(&b), "sibling self slot must be skipped");
    assert!(!inst_false.contains(&footer), "footer (other layer) must be untouched");
}

#[test]
fn legacy_no_op_when_no_view_matches() {
    let mut scene = crate::bb_scene::parse_bb_canvas(&legacy_frame_scene()).expect("parse");
    let mut inst_false: HashSet<BbNodeId> = HashSet::new();
    apply_bound_mfd_view(
        &mut scene,
        "BuildingBlocks_Canvas.MC_S_Power_Master",
        &mut Vec::new(),
        &mut inst_false,
    );
    assert!(inst_false.is_empty(), "no matching slot → no changes");
}

