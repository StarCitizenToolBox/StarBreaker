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

    // ── Known-outlier override mechanism ──────────────────────────────────

    fn platinum() -> UiSnapshotTolerance {
        UiSnapshotTolerance {
            numeric_relative: 0.01,
            font_size_relative: 0.05,
            numeric_screen_floor_ratio: 0.001,
            rgba_channel_abs: 0.05,
        }
    }

    fn base_element(identity: &str) -> UiSnapshotElement {
        UiSnapshotElement {
            identity: identity.to_string(),
            node_id: 1,
            category: UiSnapshotElementCategory::Text,
            draw_order_index: 0,
            node_type: "widget_text_field".to_string(),
            visible: true,
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 40.0,
            alpha: 1.0,
            blend_mode: None,
            asset_identity: None,
            alignment: None,
            vertical_alignment: None,
            overflow_mode: None,
            background_rgba: None,
            background_tint_token: None,
            stroke_rgba: None,
            stroke_tint_token: None,
            text_rgba: None,
            text_tint_token: None,
            icon_tint_rgba: None,
            icon_tint_token: None,
            stroke_extent: None,
            text_payload: Some("HELLO".to_string()),
            text_font_identity: None,
            text_font_size: None,
            line_spacing: None,
            primary_text_top: Some(46.0),
            primary_text_left: None,
            secondary_text_top: None,
            secondary_text_left: None,
        }
    }

    fn one_element_snapshot(element: UiSnapshotElement) -> UiScreenSnapshot {
        UiScreenSnapshot {
            schema_version: UI_SNAPSHOT_SCHEMA_VERSION,
            canvas_guid: "g".to_string(),
            canvas_name: None,
            target_width: 1920,
            target_height: 1080,
            elements: vec![element],
        }
    }

    fn number_outlier(field: &str, frozen: f32, target: f32, confidence: f32) -> UiKnownOutlier {
        UiKnownOutlier {
            target: "t".to_string(),
            identity: "1:widget_text_field".to_string(),
            field: field.to_string(),
            frozen_value: UiOutlierValue::Number(frozen),
            reference_target: UiOutlierValue::Number(target),
            confidence,
            reason: String::new(),
            source: String::new(),
        }
    }

    /// baseline primary_text_top, current primary_text_top, override target.
    fn compare_text_top(current_top: f32, outlier: &UiKnownOutlier) -> UiSnapshotComparison {
        let baseline = one_element_snapshot(base_element("1:widget_text_field"));
        let current = one_element_snapshot({
            let mut element = base_element("1:widget_text_field");
            element.primary_text_top = Some(current_top);
            element
        });
        compare_snapshots_with_overrides(&baseline, &current, platinum(), &[outlier])
    }

    #[test]
    fn outlier_movement_toward_reference_passes_with_improvement_note() {
        let outlier = number_outlier("primary_text_top", 46.0, 34.0, 2.0);
        let comparison = compare_text_top(38.0, &outlier);
        assert!(comparison.passed, "moving closer must not fail: {:?}", comparison.failures);
        assert!(
            comparison.improvements.iter().any(|note| note.contains("IMPROVEMENT")
                && note.contains("primary_text_top")),
            "expected an improvement note: {:?}",
            comparison.improvements
        );
    }

    #[test]
    fn outlier_movement_away_from_reference_fails_as_regression() {
        let outlier = number_outlier("primary_text_top", 46.0, 34.0, 2.0);
        let comparison = compare_text_top(50.0, &outlier);
        assert!(!comparison.passed, "moving away from target must fail");
        assert!(
            comparison.failures.iter().any(|failure| failure.contains("REGRESSED away")),
            "expected a regression failure: {:?}",
            comparison.failures
        );
    }

    #[test]
    fn outlier_no_change_is_neutral_pass_without_note() {
        let outlier = number_outlier("primary_text_top", 46.0, 34.0, 2.0);
        let comparison = compare_text_top(46.0, &outlier);
        assert!(comparison.passed);
        assert!(comparison.improvements.is_empty(), "no movement should not celebrate");
    }

    #[test]
    fn outlier_reaching_target_flags_graduation() {
        let outlier = number_outlier("primary_text_top", 46.0, 34.0, 2.0);
        let comparison = compare_text_top(35.0, &outlier);
        assert!(comparison.passed);
        assert!(
            comparison.improvements.iter().any(|note| note.contains("graduate")),
            "within confidence of target should suggest graduation: {:?}",
            comparison.improvements
        );
    }

    #[test]
    fn outlier_with_stale_frozen_value_fails_loudly() {
        // Registry frozen_value (99) disagrees with the live baseline (46).
        let outlier = number_outlier("primary_text_top", 99.0, 34.0, 2.0);
        let comparison = compare_text_top(38.0, &outlier);
        assert!(!comparison.passed, "stale outlier entry must fail");
        assert!(
            comparison.failures.iter().any(|failure| failure.contains("STALE")),
            "expected a STALE failure: {:?}",
            comparison.failures
        );
    }

    #[test]
    fn outlier_rgba_colour_moving_toward_reference_improves() {
        let outlier = UiKnownOutlier {
            target: "t".to_string(),
            identity: "1:widget_text_field".to_string(),
            field: "text_rgba".to_string(),
            frozen_value: UiOutlierValue::Rgba([0.73, 0.75, 0.81, 1.0]),
            reference_target: UiOutlierValue::Rgba([0.37, 0.65, 0.81, 1.0]),
            confidence: 0.02,
            reason: String::new(),
            source: String::new(),
        };
        let baseline = one_element_snapshot({
            let mut element = base_element("1:widget_text_field");
            element.text_rgba = Some([0.73, 0.75, 0.81, 1.0]);
            element
        });
        let current = one_element_snapshot({
            let mut element = base_element("1:widget_text_field");
            element.text_rgba = Some([0.50, 0.70, 0.81, 1.0]);
            element
        });
        let comparison = compare_snapshots_with_overrides(&baseline, &current, platinum(), &[&outlier]);
        assert!(comparison.passed, "colour moving toward target must pass: {:?}", comparison.failures);
        assert!(
            comparison.improvements.iter().any(|note| note.contains("IMPROVEMENT")),
            "expected colour improvement note: {:?}",
            comparison.improvements
        );
    }

    #[test]
    fn outlier_discrete_token_reaching_target_improves() {
        let outlier = UiKnownOutlier {
            target: "t".to_string(),
            identity: "1:widget_text_field".to_string(),
            field: "text_tint_token".to_string(),
            frozen_value: UiOutlierValue::Text("Bright".to_string()),
            reference_target: UiOutlierValue::Text("Base".to_string()),
            confidence: 0.0,
            reason: String::new(),
            source: String::new(),
        };
        let baseline = one_element_snapshot({
            let mut element = base_element("1:widget_text_field");
            element.text_tint_token = Some("Bright".to_string());
            element
        });
        let current = one_element_snapshot({
            let mut element = base_element("1:widget_text_field");
            element.text_tint_token = Some("Base".to_string());
            element
        });
        let comparison = compare_snapshots_with_overrides(&baseline, &current, platinum(), &[&outlier]);
        assert!(comparison.passed, "token reaching target must pass: {:?}", comparison.failures);
        assert!(comparison.improvements.iter().any(|note| note.contains("IMPROVEMENT")));
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
