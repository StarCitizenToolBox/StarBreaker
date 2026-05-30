pub fn compare_snapshots(
    baseline: &UiScreenSnapshot,
    current: &UiScreenSnapshot,
    tolerance: UiSnapshotTolerance,
) -> UiSnapshotComparison {
    let mut failures = Vec::new();
    let baseline_max_dim = baseline.target_width.max(baseline.target_height) as f32;

    let baseline_visible: HashMap<&str, &UiSnapshotElement> = baseline
        .elements
        .iter()
        .filter(|element| element.visible)
        .map(|element| (element.identity.as_str(), element))
        .collect();
    let current_visible: HashMap<&str, &UiSnapshotElement> = current
        .elements
        .iter()
        .filter(|element| element.visible)
        .map(|element| (element.identity.as_str(), element))
        .collect();

    for (identity, _) in &baseline_visible {
        if !current_visible.contains_key(identity) {
            failures.push(format!("missing visible element: {identity}"));
        }
    }
    for (identity, element) in &current_visible {
        if !baseline_visible.contains_key(identity) {
            failures.push(format!(
                "unexpected new visible element: {identity} ({:?})",
                element.category
            ));
        }
    }

    for (identity, baseline_element) in &baseline_visible {
        let Some(current_element) = current_visible.get(identity) else {
            continue;
        };

        // Category-targeted strict checks.
        if baseline_element.category != current_element.category {
            failures.push(format!(
                "{identity}: category drift baseline={:?} current={:?}",
                baseline_element.category, current_element.category
            ));
        }
        if baseline_element.draw_order_index != current_element.draw_order_index {
            failures.push(format!(
                "{identity}: draw-order drift baseline={} current={}",
                baseline_element.draw_order_index, current_element.draw_order_index
            ));
        }
        if baseline_element.blend_mode != current_element.blend_mode {
            failures.push(format!(
                "{identity}: blend mode drift baseline={:?} current={:?}",
                baseline_element.blend_mode, current_element.blend_mode
            ));
        }
        if baseline_element.asset_identity != current_element.asset_identity {
            failures.push(format!(
                "{identity}: asset identity drift baseline={:?} current={:?}",
                baseline_element.asset_identity, current_element.asset_identity
            ));
        }
        if baseline_element.alignment != current_element.alignment {
            failures.push(format!(
                "{identity}: alignment drift baseline={:?} current={:?}",
                baseline_element.alignment, current_element.alignment
            ));
        }
        if baseline_element.vertical_alignment != current_element.vertical_alignment {
            failures.push(format!(
                "{identity}: vertical alignment drift baseline={:?} current={:?}",
                baseline_element.vertical_alignment, current_element.vertical_alignment
            ));
        }
        if baseline_element.overflow_mode != current_element.overflow_mode {
            failures.push(format!(
                "{identity}: overflow drift baseline={:?} current={:?}",
                baseline_element.overflow_mode, current_element.overflow_mode
            ));
        }
        if baseline_element.text_payload != current_element.text_payload {
            failures.push(format!(
                "{identity}: text payload/case drift baseline={:?} current={:?}",
                baseline_element.text_payload, current_element.text_payload
            ));
        }
        if baseline_element.text_font_identity != current_element.text_font_identity {
            failures.push(format!(
                "{identity}: font identity drift baseline={:?} current={:?}",
                baseline_element.text_font_identity, current_element.text_font_identity
            ));
        }

        // Tolerance-based numeric checks.
        compare_geometry_numeric(
            &mut failures,
            identity,
            "x",
            baseline_element.x,
            current_element.x,
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "y",
            baseline_element.y,
            current_element.y,
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "w",
            baseline_element.w,
            current_element.w,
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "h",
            baseline_element.h,
            current_element.h,
            baseline_max_dim,
            tolerance,
        );
        compare_unit_numeric(
            &mut failures,
            identity,
            "alpha",
            baseline_element.alpha,
            current_element.alpha,
            tolerance,
        );

        compare_optional_numeric(
            &mut failures,
            identity,
            "stroke_extent",
            baseline_element.stroke_extent,
            current_element.stroke_extent,
            baseline_max_dim,
            tolerance,
        );
        compare_optional_font_numeric(
            &mut failures,
            identity,
            "line_spacing",
            baseline_element.line_spacing,
            current_element.line_spacing,
            baseline_max_dim,
            tolerance,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "background_rgba",
            baseline_element.background_rgba,
            current_element.background_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "stroke_rgba",
            baseline_element.stroke_rgba,
            current_element.stroke_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "text_rgba",
            baseline_element.text_rgba,
            current_element.text_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "icon_tint_rgba",
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

fn classify_node(node: &UiIrNode) -> Option<UiSnapshotElementCategory> {
    if node.text_payload.is_some() {
        Some(UiSnapshotElementCategory::Text)
    } else if node.asset_ref.is_some() {
        Some(UiSnapshotElementCategory::Image)
    } else if node.custom_shape.is_some() {
        Some(UiSnapshotElementCategory::Shape)
    } else {
        None
    }
}

fn blend_mode_name(mode: &UiIrColourBlendMode) -> String {
    match mode {
        UiIrColourBlendMode::SourceOver => "source_over".to_string(),
        UiIrColourBlendMode::Additive => "additive".to_string(),
    }
}

fn compare_geometry_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: f32,
    current: f32,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) {
    let threshold = hybrid_threshold(baseline, baseline_max_dim, tolerance);
    let abs_delta = (baseline - current).abs();
    if abs_delta > threshold {
        failures.push(format!(
            "{identity}: {field} drift baseline={baseline:.4} current={current:.4} delta={abs_delta:.4} threshold={threshold:.4}"
        ));
    }
}

fn compare_unit_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: f32,
    current: f32,
    tolerance: UiSnapshotTolerance,
) {
    let threshold = (baseline.abs() * tolerance.numeric_relative).max(tolerance.rgba_channel_abs);
    let abs_delta = (baseline - current).abs();
    if abs_delta > threshold {
        failures.push(format!(
            "{identity}: {field} drift baseline={baseline:.4} current={current:.4} delta={abs_delta:.4} threshold={threshold:.4}"
        ));
    }
}

fn compare_optional_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: Option<f32>,
    current: Option<f32>,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) {
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            let threshold = hybrid_threshold(base, baseline_max_dim, tolerance);
            let abs_delta = (base - cur).abs();
            if abs_delta > threshold {
                failures.push(format!(
                    "{identity}: {field} drift baseline={base:.4} current={cur:.4} delta={abs_delta:.4} threshold={threshold:.4}"
                ));
            }
        }
        (None, None) => {}
        _ => failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn compare_optional_font_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: Option<f32>,
    current: Option<f32>,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) {
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            let threshold = hybrid_threshold_with_relative(
                base,
                baseline_max_dim,
                tolerance.font_size_relative,
                tolerance.numeric_screen_floor_ratio,
            );
            let abs_delta = (base - cur).abs();
            if abs_delta > threshold {
                failures.push(format!(
                    "{identity}: {field} drift baseline={base:.4} current={cur:.4} delta={abs_delta:.4} threshold={threshold:.4}"
                ));
            }
        }
        (None, None) => {}
        _ => failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn compare_optional_rgba(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: Option<[f32; 4]>,
    current: Option<[f32; 4]>,
    channel_tolerance: f32,
) {
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            for (index, (base_ch, cur_ch)) in base.iter().zip(cur.iter()).enumerate() {
                let abs_delta = (base_ch - cur_ch).abs();
                if abs_delta > channel_tolerance {
                    failures.push(format!(
                        "{identity}: {field}[{index}] drift baseline={base_ch:.4} current={cur_ch:.4} delta={abs_delta:.4} threshold={channel_tolerance:.4}"
                    ));
                }
            }
        }
        (None, None) => {}
        _ => failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn hybrid_threshold(
    baseline: f32,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) -> f32 {
    hybrid_threshold_with_relative(
        baseline,
        baseline_max_dim,
        tolerance.numeric_relative,
        tolerance.numeric_screen_floor_ratio,
    )
}

fn hybrid_threshold_with_relative(
    baseline: f32,
    baseline_max_dim: f32,
    numeric_relative: f32,
    numeric_screen_floor_ratio: f32,
) -> f32 {
    let relative_limit = baseline.abs() * numeric_relative;
    let screen_floor_limit = baseline_max_dim * numeric_screen_floor_ratio;
    relative_limit.max(screen_floor_limit)
}

