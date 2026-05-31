#[cfg(test)]
mod tests {
    use super::*;
    use super::tests_support::{base_document, base_node};

    #[test]
    fn snapshot_extracts_visible_text_elements_with_draw_order() {
        let low_layer = base_node(2, 1);
        let high_layer = base_node(1, 9);
        let document = base_document(vec![high_layer, low_layer]);

        let snapshot = snapshot_from_ui_ir(&document);
        assert_eq!(snapshot.schema_version, UI_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snapshot.elements.len(), 2);
        assert_eq!(snapshot.elements[0].node_id, 2);
        assert_eq!(snapshot.elements[0].draw_order_index, 0);
        assert_eq!(snapshot.elements[1].node_id, 1);
        assert_eq!(snapshot.elements[1].draw_order_index, 1);
    }

    #[test]
    fn compare_snapshots_fails_on_draw_order_drift() {
        let baseline = base_document(vec![base_node(1, 1), base_node(2, 2)]);
        let current = base_document(vec![base_node(1, 2), base_node(2, 1)]);

        let baseline_snapshot = snapshot_from_ui_ir(&baseline);
        let current_snapshot = snapshot_from_ui_ir(&current);
        let comparison = compare_snapshots(&baseline_snapshot, &current_snapshot, UiSnapshotTolerance::default());

        assert!(!comparison.passed);
        assert!(
            comparison
                .failures
                .iter()
                .any(|failure| failure.contains("draw-order drift"))
        );
    }

    #[test]
    fn compare_snapshots_fails_on_new_visible_shape_element() {
        let baseline_node = base_node(1, 1);
        let baseline = base_document(vec![baseline_node]);

        let mut shape_node = base_node(2, 2);
        shape_node.text_payload = None;
        shape_node.text_style = None;
        shape_node.node_type = "widget_custom_shape".to_string();
        shape_node.custom_shape = Some(crate::ui_ir::UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        let current = base_document(vec![base_node(1, 1), shape_node]);

        let baseline_snapshot = snapshot_from_ui_ir(&baseline);
        let current_snapshot = snapshot_from_ui_ir(&current);
        let comparison = compare_snapshots(&baseline_snapshot, &current_snapshot, UiSnapshotTolerance::default());

        assert!(!comparison.passed);
        assert!(
            comparison
                .failures
                .iter()
                .any(|failure| failure.contains("unexpected new visible element"))
        );
    }

    #[test]
    fn compare_snapshots_uses_font_size_relative_for_line_spacing() {
        let baseline = base_document(vec![base_node(1, 1)]);
        let mut current = baseline.clone();
        current.nodes[0]
            .text_style
            .as_mut()
            .expect("text style should exist")
            .line_spacing = Some(2.08);

        let baseline_snapshot = snapshot_from_ui_ir(&baseline);
        let current_snapshot = snapshot_from_ui_ir(&current);
        let comparison = compare_snapshots(
            &baseline_snapshot,
            &current_snapshot,
            UiSnapshotTolerance {
                numeric_relative: 0.01,
                font_size_relative: 0.05,
                numeric_screen_floor_ratio: 0.001,
                rgba_channel_abs: 0.05,
            },
        );

        assert!(
            comparison.passed,
            "line_spacing drift within font-size tolerance should pass: {:?}",
            comparison.failures
        );
    }

    #[test]
    fn compare_snapshots_fails_on_explicit_font_size_drift() {
        let baseline = base_document(vec![base_node(1, 1)]);
        let mut current = baseline.clone();
        current.nodes[0]
            .text_style
            .as_mut()
            .expect("text style should exist")
            .font_size = crate::ui_ir::UiIrValue::Fixed { value: 40.0 };

        let baseline_snapshot = snapshot_from_ui_ir(&baseline);
        let current_snapshot = snapshot_from_ui_ir(&current);
        let comparison = compare_snapshots(
            &baseline_snapshot,
            &current_snapshot,
            UiSnapshotTolerance {
                numeric_relative: 0.01,
                font_size_relative: 0.05,
                numeric_screen_floor_ratio: 0.001,
                rgba_channel_abs: 0.05,
            },
        );

        assert!(!comparison.passed, "font_size drift should fail comparator");
        assert!(
            comparison
                .failures
                .iter()
                .any(|failure| failure.contains("font_size drift")),
            "failure output should include font_size drift"
        );
    }

    #[test]
    fn renderer_metadata_snapshot_is_deterministic() {
        let document = base_document(vec![base_node(1, 1), base_node(2, 2)]);
        let first = renderer_metadata_snapshot_from_ui_ir(&document);
        let second = renderer_metadata_snapshot_from_ui_ir(&document);
        assert_eq!(first, second);
    }

    #[test]
    fn renderer_metadata_comparator_fails_on_blend_drift() {
        let baseline = base_document(vec![base_node(1, 1)]);
        let mut current = baseline.clone();
        current.nodes[0].colour_blend_mode = Some(crate::ui_ir::UiIrColourBlendMode::Additive);

        let baseline_snapshot = renderer_metadata_snapshot_from_ui_ir(&baseline);
        let current_snapshot = renderer_metadata_snapshot_from_ui_ir(&current);
        let comparison = compare_renderer_metadata_snapshots(
            &baseline_snapshot,
            &current_snapshot,
            UiSnapshotTolerance::default(),
        );

        assert!(!comparison.passed);
        assert!(
            comparison
                .failures
                .iter()
                .any(|failure| failure.contains("renderer blend drift"))
        );
    }
}
