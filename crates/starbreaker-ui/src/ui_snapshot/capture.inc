pub fn snapshot_from_ui_ir(document: &UiIrDocument) -> UiScreenSnapshot {
    let mut draw_nodes: Vec<&UiIrNode> = document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .collect();
    draw_nodes.sort_by_key(|node| (node.layer, node.id));

    let mut elements = Vec::new();
    for (draw_order_index, node) in draw_nodes.into_iter().enumerate() {
        let Some(category) = classify_node(node) else {
            continue;
        };
        elements.push(UiSnapshotElement {
            identity: format!("{}:{}", node.id, node.node_type),
            node_id: node.id,
            category,
            draw_order_index: draw_order_index as u32,
            node_type: node.node_type.clone(),
            visible: node.is_active,
            x: node.computed_rect.x,
            y: node.computed_rect.y,
            w: node.computed_rect.w,
            h: node.computed_rect.h,
            alpha: node.alpha,
            blend_mode: node.colour_blend_mode.as_ref().map(blend_mode_name),
            asset_identity: node
                .asset_ref
                .clone()
                .or_else(|| node.custom_shape.as_ref().and_then(|shape| shape.svg_path.clone())),
            alignment: node.text_style.as_ref().map(|style| style.alignment.clone()),
            vertical_alignment: node
                .text_style
                .as_ref()
                .map(|style| style.vertical_alignment.clone()),
            overflow_mode: node.overflow_mode.clone(),
            background_rgba: node.background_fill_colour,
            background_tint_token: node.background_fill_colour_token.clone(),
            stroke_rgba: node.stroke_colour,
            stroke_tint_token: node.stroke_colour_token.clone(),
            text_rgba: node.text_style.as_ref().and_then(|style| style.colour),
            text_tint_token: node
                .text_style
                .as_ref()
                .and_then(|style| style.colour_token.clone()),
            icon_tint_rgba: node.icon_tint_colour,
            icon_tint_token: node.icon_tint_colour_token.clone(),
            stroke_extent: node.stroke_extent,
            text_payload: node.text_payload.as_ref().and_then(|payload| match payload {
                crate::ui_ir::UiIrTextPayload::Resolved { text } => Some(text.clone()),
                crate::ui_ir::UiIrTextPayload::UnresolvedKey { key } => Some(key.clone()),
                crate::ui_ir::UiIrTextPayload::IntentionallyEmpty { key } => key.clone(),
                crate::ui_ir::UiIrTextPayload::Empty => None,
            }),
            text_font_identity: node.text_style.as_ref().and_then(|style| {
                style
                    .font_record
                    .clone()
                    .or_else(|| style.resolved_font_record.as_ref().map(|record| record.to_string()))
            }),
            text_font_size: node.text_style.as_ref().and_then(|style| match style.font_size {
                crate::ui_ir::UiIrValue::Fixed { value } if value.is_finite() && value > 0.0 => {
                    Some(value)
                }
                _ => None,
            }),
            line_spacing: node.text_style.as_ref().and_then(|style| style.line_spacing),
        });
    }

    UiScreenSnapshot {
        schema_version: UI_SNAPSHOT_SCHEMA_VERSION,
        canvas_guid: document.canvas_guid.clone(),
        canvas_name: document.canvas_name.clone(),
        target_width: document.target_width,
        target_height: document.target_height,
        elements,
    }
}

