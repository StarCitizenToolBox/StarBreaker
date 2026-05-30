#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRendererMetadataSnapshot {
    pub schema_version: u32,
    pub canvas_guid: String,
    pub target_width: u32,
    pub target_height: u32,
    pub elements: Vec<UiRendererMetadataElement>,
}

/// One visible renderer element in draw order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRendererMetadataElement {
    pub identity: String,
    pub node_id: u32,
    pub node_type: String,
    pub draw_order_index: u32,
    pub blend_mode: Option<String>,
    pub asset_identity: Option<String>,
    pub rect: [f32; 4],
    pub background_rgba: Option<[f32; 4]>,
    pub stroke_rgba: Option<[f32; 4]>,
    pub text_rgba: Option<[f32; 4]>,
    pub icon_tint_rgba: Option<[f32; 4]>,
}

/// Extract low-cost renderer metadata without pixel snapshots.
pub fn renderer_metadata_snapshot_from_ui_ir(document: &UiIrDocument) -> UiRendererMetadataSnapshot {
    let mut draw_nodes: Vec<&UiIrNode> = document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .collect();
    draw_nodes.sort_by_key(|node| (node.layer, node.id));

    let mut elements = Vec::new();
    for (draw_order_index, node) in draw_nodes.into_iter().enumerate() {
        let rect = crate::ir_compose::debug_node_draw_rect(node, document);
        elements.push(UiRendererMetadataElement {
            identity: format!("{}:{}", node.id, node.node_type),
            node_id: node.id,
            node_type: node.node_type.clone(),
            draw_order_index: draw_order_index as u32,
            blend_mode: node.colour_blend_mode.as_ref().map(blend_mode_name),
            asset_identity: node
                .asset_ref
                .clone()
                .or_else(|| node.custom_shape.as_ref().and_then(|shape| shape.svg_path.clone())),
            rect: [rect.x, rect.y, rect.w, rect.h],
            background_rgba: node.background_fill_colour,
            stroke_rgba: node.stroke_colour,
            text_rgba: node.text_style.as_ref().and_then(|style| style.colour),
            icon_tint_rgba: node.icon_tint_colour,
        });
    }

    UiRendererMetadataSnapshot {
        schema_version: UI_SNAPSHOT_SCHEMA_VERSION,
        canvas_guid: document.canvas_guid.clone(),
        target_width: document.target_width,
        target_height: document.target_height,
        elements,
    }
}

/// Compare renderer metadata snapshots using Phase 7 tolerance policy.
pub fn compare_renderer_metadata_snapshots(
    baseline: &UiRendererMetadataSnapshot,
    current: &UiRendererMetadataSnapshot,
    tolerance: UiSnapshotTolerance,
) -> UiSnapshotComparison {
    let mut failures = Vec::new();
    let baseline_max_dim = baseline.target_width.max(baseline.target_height) as f32;

    let baseline_by_identity: HashMap<&str, &UiRendererMetadataElement> = baseline
        .elements
        .iter()
        .map(|element| (element.identity.as_str(), element))
        .collect();
    let current_by_identity: HashMap<&str, &UiRendererMetadataElement> = current
        .elements
        .iter()
        .map(|element| (element.identity.as_str(), element))
        .collect();

    for (identity, _) in &baseline_by_identity {
        if !current_by_identity.contains_key(identity) {
            failures.push(format!("missing renderer metadata element: {identity}"));
        }
    }
    for (identity, element) in &current_by_identity {
        if !baseline_by_identity.contains_key(identity) {
            failures.push(format!(
                "unexpected renderer metadata element: {identity} ({})",
                element.node_type
            ));
        }
    }

    for (identity, baseline_element) in &baseline_by_identity {
        let Some(current_element) = current_by_identity.get(identity) else {
            continue;
        };
        if baseline_element.draw_order_index != current_element.draw_order_index {
            failures.push(format!(
                "{identity}: renderer draw-order drift baseline={} current={}",
                baseline_element.draw_order_index, current_element.draw_order_index
            ));
        }
        if baseline_element.blend_mode != current_element.blend_mode {
            failures.push(format!(
                "{identity}: renderer blend drift baseline={:?} current={:?}",
                baseline_element.blend_mode, current_element.blend_mode
            ));
        }
        if baseline_element.asset_identity != current_element.asset_identity {
            failures.push(format!(
                "{identity}: renderer asset identity drift baseline={:?} current={:?}",
                baseline_element.asset_identity, current_element.asset_identity
            ));
        }

        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_x",
            baseline_element.rect[0],
            current_element.rect[0],
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_y",
            baseline_element.rect[1],
            current_element.rect[1],
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_w",
            baseline_element.rect[2],
            current_element.rect[2],
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_h",
            baseline_element.rect[3],
            current_element.rect[3],
            baseline_max_dim,
            tolerance,
        );

        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_background_rgba",
            baseline_element.background_rgba,
            current_element.background_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_stroke_rgba",
            baseline_element.stroke_rgba,
            current_element.stroke_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_text_rgba",
            baseline_element.text_rgba,
            current_element.text_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_icon_tint_rgba",
            baseline_element.icon_tint_rgba,
            current_element.icon_tint_rgba,
            tolerance.rgba_channel_abs,
        );
    }

    UiSnapshotComparison {
        passed: failures.is_empty(),
        failures,
    }
}

