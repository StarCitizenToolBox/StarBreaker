use starbreaker_ui::{
    UiIrDocument, UiRegressionManifest, UiScreenSnapshot, UiSnapshotTolerance,
    compare_manifest_targets_with_loader, compare_snapshots, snapshot_from_ui_ir,
};
use std::collections::HashMap;

fn medical1_ir() -> UiIrDocument {
    serde_json::from_str(include_str!("fixtures/medical_ir/medical1-screen_16x9_a-ir.json"))
        .expect("medical1 IR fixture should parse")
}

fn medical2_ir() -> UiIrDocument {
    serde_json::from_str(include_str!(
        "fixtures/medical_ir/medical2-mesh_end_screen_plane-ir.json"
    ))
    .expect("medical2 IR fixture should parse")
}

fn medical_manifest() -> UiRegressionManifest {
    serde_json::from_str(include_str!("fixtures/medical_ir/medical_snapshot_manifest.json"))
        .expect("medical regression manifest fixture should parse")
}

fn manifest_snapshot_lookup() -> HashMap<String, UiScreenSnapshot> {
    HashMap::from([
        ("medical1.baseline".to_string(), snapshot_from_ui_ir(&medical1_ir())),
        ("medical1.current".to_string(), snapshot_from_ui_ir(&medical1_ir())),
        ("medical2.baseline".to_string(), snapshot_from_ui_ir(&medical2_ir())),
        ("medical2.current".to_string(), snapshot_from_ui_ir(&medical2_ir())),
    ])
}

#[test]
fn medical_snapshots_are_deterministic_for_phase1_fixtures() {
    for document in [medical1_ir(), medical2_ir()] {
        let first = snapshot_from_ui_ir(&document);
        let second = snapshot_from_ui_ir(&document);
        assert_eq!(first, second, "snapshot extraction must be deterministic");
    }
}

#[test]
fn medical_manifest_targets_pass_for_phase1_fixtures() {
    let manifest = medical_manifest();
    let snapshots = manifest_snapshot_lookup();

    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot fixture for {path}"))
    })
    .expect("manifest runner should load all medical fixtures");

    assert_eq!(results.len(), 2, "expected two medical targets");
    for result in results {
        assert!(
            result.comparison.passed,
            "manifest target {} should pass baseline comparison: {:?}",
            result.id,
            result.comparison.failures
        );
    }
}

#[test]
fn medical_snapshot_comparator_flags_text_case_drift() {
    let baseline_doc = medical1_ir();
    let mut current_doc = baseline_doc.clone();

    let text_node = current_doc
        .nodes
        .iter_mut()
        .find(|node| {
            node.is_active
                && matches!(
                    node.text_payload,
                    Some(starbreaker_ui::UiIrTextPayload::Resolved { .. })
                )
        })
        .expect("fixture should include active resolved text");
    if let Some(starbreaker_ui::UiIrTextPayload::Resolved { text }) = text_node.text_payload.as_mut() {
        let lowered = text.to_ascii_lowercase();
        *text = if lowered == *text {
            format!("{text}x")
        } else {
            lowered
        };
    }

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = compare_snapshots(&baseline, &current, UiSnapshotTolerance::default());

    assert!(!comparison.passed, "case drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("text payload/case drift")),
        "failure output should mention text payload/case drift"
    );
}

#[test]
fn medical_snapshot_comparator_flags_new_visible_shape() {
    let baseline_doc = medical2_ir();
    let mut current_doc = baseline_doc.clone();

    let mut new_node = current_doc
        .nodes
        .iter()
        .find(|node| node.is_active)
        .expect("fixture should include active nodes")
        .clone();
    new_node.id = 999_999;
    new_node.node_type = "widget_custom_shape".to_string();
    new_node.text_payload = None;
    new_node.text_style = None;
    new_node.asset_ref = None;
    new_node.custom_shape = Some(starbreaker_ui::ui_ir::UiIrCustomShape {
        shape_type: Some("svg".to_string()),
        shape: None,
        svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
        render_shape: Some(true),
        enable_nine_slice_rect: None,
        nine_slice_rect: None,
        nine_slice_scale: None,
    });
    current_doc.nodes.push(new_node);

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = compare_snapshots(&baseline, &current, UiSnapshotTolerance::default());

    assert!(
        !comparison.passed,
        "new visible shape should fail comparator"
    );
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("unexpected new visible element")),
        "failure output should mention unexpected new visible element"
    );
}

#[test]
fn medical_snapshot_comparator_flags_geometry_drift() {
    let baseline_doc = medical1_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| {
            node.is_active
                && (node.text_payload.is_some() || node.asset_ref.is_some() || node.custom_shape.is_some())
        })
        .expect("fixture should include active tracked nodes");
    node.computed_rect.x += 60.0;

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = compare_snapshots(&baseline, &current, UiSnapshotTolerance::default());

    assert!(!comparison.passed, "geometry drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains(" x drift ") || line.contains(": x drift")),
        "failure output should include x geometry drift"
    );
}

#[test]
fn medical_snapshot_comparator_flags_colour_drift() {
    let baseline_doc = medical2_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| node.is_active && node.text_style.is_some())
        .expect("fixture should include active styled text");
    let style = node.text_style.as_mut().expect("text style expected");
    style.colour = Some([1.0, 0.0, 0.0, 1.0]);

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = compare_snapshots(&baseline, &current, UiSnapshotTolerance::default());

    assert!(!comparison.passed, "colour drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("text_rgba") || line.contains("icon_tint_rgba") || line.contains("background_rgba")),
        "failure output should include colour-channel drift"
    );
}

#[test]
fn medical_snapshot_comparator_flags_font_identity_drift() {
    let baseline_doc = medical1_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| node.is_active && node.text_style.is_some())
        .expect("fixture should include active styled text");
    let style = node.text_style.as_mut().expect("text style expected");
    style.font_record = Some("$Text1Bold".to_string());

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = compare_snapshots(&baseline, &current, UiSnapshotTolerance::default());

    assert!(!comparison.passed, "font identity drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("font identity drift")),
        "failure output should include font identity drift"
    );
}

#[test]
fn medical_snapshot_comparator_flags_draw_order_drift() {
    let baseline_doc = medical2_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| node.is_active)
        .expect("fixture should include active nodes");
    node.layer += 100;

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = compare_snapshots(&baseline, &current, UiSnapshotTolerance::default());

    assert!(!comparison.passed, "draw-order drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("draw-order drift")),
        "failure output should include draw-order drift"
    );
}
