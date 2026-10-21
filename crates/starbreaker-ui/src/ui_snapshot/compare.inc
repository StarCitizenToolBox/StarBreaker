/// Compare two screen snapshots with no known-outlier overrides (strict).
pub fn compare_snapshots(
    baseline: &UiScreenSnapshot,
    current: &UiScreenSnapshot,
    tolerance: UiSnapshotTolerance,
) -> UiSnapshotComparison {
    compare_snapshots_with_overrides(baseline, current, tolerance, &[])
}

/// Compare two screen snapshots, applying reference-anchored known-outlier
/// overrides.
///
/// For a field with a matching override, the check becomes one-sided: a value
/// that moved *toward* the override's `reference_target` (closer than the frozen
/// baseline) passes and is recorded as an improvement; a value that moved
/// *away* fails as a regression. Fields without an override use the normal
/// symmetric tolerance.
pub fn compare_snapshots_with_overrides(
    baseline: &UiScreenSnapshot,
    current: &UiScreenSnapshot,
    tolerance: UiSnapshotTolerance,
    overrides: &[&UiKnownOutlier],
) -> UiSnapshotComparison {
    let mut ctx = CompareCtx::new(overrides);
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
            ctx.failures.push(format!("missing visible element: {identity}"));
        }
    }
    for (identity, element) in &current_visible {
        if !baseline_visible.contains_key(identity) {
            ctx.failures.push(format!(
                "unexpected new visible element: {identity} ({:?})",
                element.category
            ));
        }
    }

    for (identity, baseline_element) in &baseline_visible {
        let Some(current_element) = current_visible.get(identity) else {
            continue;
        };

        // Category-targeted strict checks (not overrideable).
        if baseline_element.category != current_element.category {
            ctx.failures.push(format!(
                "{identity}: category drift baseline={:?} current={:?}",
                baseline_element.category, current_element.category
            ));
        }
        if baseline_element.draw_order_index != current_element.draw_order_index {
            ctx.failures.push(format!(
                "{identity}: draw-order drift baseline={} current={}",
                baseline_element.draw_order_index, current_element.draw_order_index
            ));
        }
        if baseline_element.blend_mode != current_element.blend_mode {
            ctx.failures.push(format!(
                "{identity}: blend mode drift baseline={:?} current={:?}",
                baseline_element.blend_mode, current_element.blend_mode
            ));
        }
        if baseline_element.asset_identity != current_element.asset_identity {
            ctx.failures.push(format!(
                "{identity}: asset identity drift baseline={:?} current={:?}",
                baseline_element.asset_identity, current_element.asset_identity
            ));
        }
        if baseline_element.alignment != current_element.alignment {
            ctx.failures.push(format!(
                "{identity}: alignment drift baseline={:?} current={:?}",
                baseline_element.alignment, current_element.alignment
            ));
        }
        if baseline_element.vertical_alignment != current_element.vertical_alignment {
            ctx.failures.push(format!(
                "{identity}: vertical alignment drift baseline={:?} current={:?}",
                baseline_element.vertical_alignment, current_element.vertical_alignment
            ));
        }
        if baseline_element.overflow_mode != current_element.overflow_mode {
            ctx.failures.push(format!(
                "{identity}: overflow drift baseline={:?} current={:?}",
                baseline_element.overflow_mode, current_element.overflow_mode
            ));
        }
        if baseline_element.text_payload != current_element.text_payload {
            ctx.failures.push(format!(
                "{identity}: text payload/case drift baseline={:?} current={:?}",
                baseline_element.text_payload, current_element.text_payload
            ));
        }
        if baseline_element.text_font_identity != current_element.text_font_identity {
            ctx.failures.push(format!(
                "{identity}: font identity/font_weight drift baseline={:?} current={:?}",
                baseline_element.text_font_identity, current_element.text_font_identity
            ));
        }

        // Tolerance-based numeric checks (overrideable).
        compare_geometry_numeric(
            &mut ctx, identity, "x", baseline_element.x, current_element.x, baseline_max_dim, tolerance,
        );
        compare_geometry_numeric(
            &mut ctx, identity, "y", baseline_element.y, current_element.y, baseline_max_dim, tolerance,
        );
        compare_geometry_numeric(
            &mut ctx, identity, "w", baseline_element.w, current_element.w, baseline_max_dim, tolerance,
        );
        compare_geometry_numeric(
            &mut ctx, identity, "h", baseline_element.h, current_element.h, baseline_max_dim, tolerance,
        );
        compare_unit_numeric(
            &mut ctx, identity, "alpha", baseline_element.alpha, current_element.alpha, tolerance,
        );

        compare_optional_numeric(
            &mut ctx, identity, "stroke_extent",
            baseline_element.stroke_extent, current_element.stroke_extent, baseline_max_dim, tolerance,
        );
        compare_optional_numeric(
            &mut ctx, identity, "primary_text_top",
            baseline_element.primary_text_top, current_element.primary_text_top, baseline_max_dim, tolerance,
        );
        compare_optional_numeric(
            &mut ctx, identity, "primary_text_left",
            baseline_element.primary_text_left, current_element.primary_text_left, baseline_max_dim, tolerance,
        );
        compare_optional_numeric(
            &mut ctx, identity, "secondary_text_top",
            baseline_element.secondary_text_top, current_element.secondary_text_top, baseline_max_dim, tolerance,
        );
        compare_optional_numeric(
            &mut ctx, identity, "secondary_text_left",
            baseline_element.secondary_text_left, current_element.secondary_text_left, baseline_max_dim, tolerance,
        );
        compare_optional_font_numeric(
            &mut ctx, identity, "font_size",
            baseline_element.text_font_size, current_element.text_font_size, baseline_max_dim, tolerance,
        );
        compare_optional_font_numeric(
            &mut ctx, identity, "line_spacing",
            baseline_element.line_spacing, current_element.line_spacing, baseline_max_dim, tolerance,
        );
        compare_optional_rgba(
            &mut ctx, identity, "background_rgba",
            baseline_element.background_rgba, current_element.background_rgba, tolerance,
        );
        compare_optional_string(
            &mut ctx, identity, "background_tint_token",
            baseline_element.background_tint_token.as_deref(),
            current_element.background_tint_token.as_deref(),
        );
        compare_optional_rgba(
            &mut ctx, identity, "stroke_rgba",
            baseline_element.stroke_rgba, current_element.stroke_rgba, tolerance,
        );
        compare_optional_string(
            &mut ctx, identity, "stroke_tint_token",
            baseline_element.stroke_tint_token.as_deref(),
            current_element.stroke_tint_token.as_deref(),
        );
        compare_optional_rgba(
            &mut ctx, identity, "text_rgba",
            baseline_element.text_rgba, current_element.text_rgba, tolerance,
        );
        compare_optional_string(
            &mut ctx, identity, "text_tint_token",
            baseline_element.text_tint_token.as_deref(),
            current_element.text_tint_token.as_deref(),
        );
        compare_optional_rgba(
            &mut ctx, identity, "icon_tint_rgba",
            baseline_element.icon_tint_rgba, current_element.icon_tint_rgba, tolerance,
        );
        compare_optional_string(
            &mut ctx, identity, "icon_tint_token",
            baseline_element.icon_tint_token.as_deref(),
            current_element.icon_tint_token.as_deref(),
        );
    }

    UiSnapshotComparison {
        passed: ctx.failures.is_empty(),
        failures: ctx.failures,
        improvements: ctx.improvements,
    }
}

/// Comparison accumulator: collects failures + positive-reinforcement notes and
/// holds the known-outlier overrides indexed by `(identity, field)`.
struct CompareCtx<'a> {
    failures: Vec<String>,
    improvements: Vec<String>,
    overrides: HashMap<(&'a str, &'a str), &'a UiKnownOutlier>,
}

impl<'a> CompareCtx<'a> {
    fn new(overrides: &'a [&'a UiKnownOutlier]) -> Self {
        let overrides = overrides
            .iter()
            .map(|outlier| ((outlier.identity.as_str(), outlier.field.as_str()), *outlier))
            .collect();
        Self {
            failures: Vec::new(),
            improvements: Vec::new(),
            overrides,
        }
    }

    fn override_for(&self, identity: &str, field: &str) -> Option<&'a UiKnownOutlier> {
        self.overrides.get(&(identity, field)).copied()
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
    ctx: &mut CompareCtx,
    identity: &str,
    field: &str,
    baseline: f32,
    current: f32,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) {
    if let Some(outlier) = ctx.override_for(identity, field) {
        let move_eps = hybrid_threshold(baseline, baseline_max_dim, tolerance);
        apply_numeric_override(ctx, identity, field, outlier, baseline, current, move_eps);
        return;
    }
    let threshold = hybrid_threshold(baseline, baseline_max_dim, tolerance);
    let abs_delta = (baseline - current).abs();
    if abs_delta > threshold {
        ctx.failures.push(format!(
            "{identity}: {field} drift baseline={baseline:.4} current={current:.4} delta={abs_delta:.4} threshold={threshold:.4}"
        ));
    }
}

fn compare_unit_numeric(
    ctx: &mut CompareCtx,
    identity: &str,
    field: &str,
    baseline: f32,
    current: f32,
    tolerance: UiSnapshotTolerance,
) {
    let threshold = (baseline.abs() * tolerance.numeric_relative).max(tolerance.rgba_channel_abs);
    if let Some(outlier) = ctx.override_for(identity, field) {
        apply_numeric_override(ctx, identity, field, outlier, baseline, current, threshold);
        return;
    }
    let abs_delta = (baseline - current).abs();
    if abs_delta > threshold {
        ctx.failures.push(format!(
            "{identity}: {field} drift baseline={baseline:.4} current={current:.4} delta={abs_delta:.4} threshold={threshold:.4}"
        ));
    }
}

fn compare_optional_numeric(
    ctx: &mut CompareCtx,
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
            if let Some(outlier) = ctx.override_for(identity, field) {
                apply_numeric_override(ctx, identity, field, outlier, base, cur, threshold);
                return;
            }
            let abs_delta = (base - cur).abs();
            if abs_delta > threshold {
                ctx.failures.push(format!(
                    "{identity}: {field} drift baseline={base:.4} current={cur:.4} delta={abs_delta:.4} threshold={threshold:.4}"
                ));
            }
        }
        (None, None) => {}
        _ => ctx.failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn compare_optional_font_numeric(
    ctx: &mut CompareCtx,
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
            if let Some(outlier) = ctx.override_for(identity, field) {
                apply_numeric_override(ctx, identity, field, outlier, base, cur, threshold);
                return;
            }
            let abs_delta = (base - cur).abs();
            if abs_delta > threshold {
                ctx.failures.push(format!(
                    "{identity}: {field} drift baseline={base:.4} current={cur:.4} delta={abs_delta:.4} threshold={threshold:.4}"
                ));
            }
        }
        (None, None) => {}
        _ => ctx.failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn compare_optional_rgba(
    ctx: &mut CompareCtx,
    identity: &str,
    field: &str,
    baseline: Option<[f32; 4]>,
    current: Option<[f32; 4]>,
    tolerance: UiSnapshotTolerance,
) {
    let channel_tolerance = tolerance.rgba_channel_abs;
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            if let Some(outlier) = ctx.override_for(identity, field) {
                apply_rgba_override(ctx, identity, field, outlier, base, cur, channel_tolerance);
                return;
            }
            for (index, (base_ch, cur_ch)) in base.iter().zip(cur.iter()).enumerate() {
                let abs_delta = (base_ch - cur_ch).abs();
                if abs_delta > channel_tolerance {
                    ctx.failures.push(format!(
                        "{identity}: {field}[{index}] drift baseline={base_ch:.4} current={cur_ch:.4} delta={abs_delta:.4} threshold={channel_tolerance:.4}"
                    ));
                }
            }
        }
        (None, None) => {}
        _ => ctx.failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn compare_optional_string(
    ctx: &mut CompareCtx,
    identity: &str,
    field: &str,
    baseline: Option<&str>,
    current: Option<&str>,
) {
    if let Some(outlier) = ctx.override_for(identity, field) {
        apply_string_override(ctx, identity, field, outlier, baseline, current);
        return;
    }
    match (baseline, current) {
        (Some(base), Some(cur)) if base != cur => ctx.failures.push(format!(
            "{identity}: {field} drift baseline={base:?} current={cur:?}"
        )),
        (Some(base), None) => ctx.failures.push(format!(
            "{identity}: {field} drift baseline={base:?} current={current:?}"
        )),
        _ => {}
    }
}

/// One-sided numeric override: passes (and celebrates) movement toward the
/// reference target; fails movement away. `move_eps` is the neutral band (the
/// field's normal threshold) — movement smaller than this is treated as noise.
fn apply_numeric_override(
    ctx: &mut CompareCtx,
    identity: &str,
    field: &str,
    outlier: &UiKnownOutlier,
    baseline: f32,
    current: f32,
    move_eps: f32,
) {
    let (UiOutlierValue::Number(frozen), UiOutlierValue::Number(target)) =
        (&outlier.frozen_value, &outlier.reference_target)
    else {
        ctx.failures.push(format!(
            "{identity}: {field} known-outlier value type mismatch (expected numbers)"
        ));
        return;
    };
    let (frozen, target) = (*frozen, *target);
    let stale_eps = move_eps.max(0.5);
    if (frozen - baseline).abs() > stale_eps {
        ctx.failures.push(format!(
            "{identity}: {field} STALE known-outlier — registry frozen_value={frozen:.3} but live baseline={baseline:.3}; re-measure/re-freeze the outlier entry"
        ));
        return;
    }
    let frozen_dist = (frozen - target).abs();
    let current_dist = (current - target).abs();
    let move_eps = move_eps.max(0.25);
    if current_dist > frozen_dist + move_eps {
        ctx.failures.push(format!(
            "{identity}: {field} REGRESSED away from reference target {target:.3}: frozen={frozen:.3} current={current:.3} (distance {frozen_dist:.3} -> {current_dist:.3})"
        ));
    } else if current_dist + move_eps < frozen_dist {
        ctx.improvements.push(format!(
            "\u{2705} IMPROVEMENT {identity}.{field}: {frozen:.3} -> {current:.3} (reference target {target:.3}, distance {frozen_dist:.3} -> {current_dist:.3}) \u{2014} closer to the in-game reference than the frozen baseline; consider re-freezing"
        ));
        let graduate_eps = outlier.confidence.max(stale_eps);
        if current_dist <= graduate_eps {
            ctx.improvements.push(format!(
                "   \u{21b3} {identity}.{field} is within \u{00b1}{graduate_eps:.3} of the reference target \u{2014} ready to graduate to a strict freeze and drop the outlier entry"
            ));
        }
    }
}

/// One-sided RGBA override using Euclidean colour distance toward the target.
fn apply_rgba_override(
    ctx: &mut CompareCtx,
    identity: &str,
    field: &str,
    outlier: &UiKnownOutlier,
    baseline: [f32; 4],
    current: [f32; 4],
    channel_tolerance: f32,
) {
    let (UiOutlierValue::Rgba(frozen), UiOutlierValue::Rgba(target)) =
        (&outlier.frozen_value, &outlier.reference_target)
    else {
        ctx.failures.push(format!(
            "{identity}: {field} known-outlier value type mismatch (expected RGBA arrays)"
        ));
        return;
    };
    let stale_eps = channel_tolerance.max(0.02) * 4.0;
    if rgba_distance(*frozen, baseline) > stale_eps {
        ctx.failures.push(format!(
            "{identity}: {field} STALE known-outlier — registry frozen_value={frozen:?} but live baseline={baseline:?}; re-measure/re-freeze the outlier entry"
        ));
        return;
    }
    let frozen_dist = rgba_distance(*frozen, *target);
    let current_dist = rgba_distance(current, *target);
    let move_eps = channel_tolerance.max(0.01);
    if current_dist > frozen_dist + move_eps {
        ctx.failures.push(format!(
            "{identity}: {field} REGRESSED away from reference colour {target:?}: frozen={frozen:?} current={current:?} (distance {frozen_dist:.3} -> {current_dist:.3})"
        ));
    } else if current_dist + move_eps < frozen_dist {
        ctx.improvements.push(format!(
            "\u{2705} IMPROVEMENT {identity}.{field}: {frozen:?} -> {current:?} (reference colour {target:?}, distance {frozen_dist:.3} -> {current_dist:.3}) \u{2014} closer to the in-game reference than the frozen baseline; consider re-freezing"
        ));
        let graduate_eps = outlier.confidence.max(stale_eps);
        if current_dist <= graduate_eps {
            ctx.improvements.push(format!(
                "   \u{21b3} {identity}.{field} is within {graduate_eps:.3} of the reference colour \u{2014} ready to graduate to a strict freeze"
            ));
        }
    }
}

/// Discrete (string/token) override: passes at either the frozen value or the
/// reference target; reaching the target emits the improvement note.
fn apply_string_override(
    ctx: &mut CompareCtx,
    identity: &str,
    field: &str,
    outlier: &UiKnownOutlier,
    baseline: Option<&str>,
    current: Option<&str>,
) {
    let UiOutlierValue::Text(target) = &outlier.reference_target else {
        ctx.failures.push(format!(
            "{identity}: {field} known-outlier value type mismatch (expected string)"
        ));
        return;
    };
    let frozen = match &outlier.frozen_value {
        UiOutlierValue::Text(frozen) => Some(frozen.as_str()),
        _ => None,
    };
    if let (Some(frozen), Some(base)) = (frozen, baseline) {
        if frozen != base {
            ctx.failures.push(format!(
                "{identity}: {field} STALE known-outlier — registry frozen_value={frozen:?} but live baseline={base:?}; re-measure/re-freeze the outlier entry"
            ));
            return;
        }
    }
    let target = target.as_str();
    match current {
        Some(cur) if cur == target => {
            if frozen != Some(target) {
                ctx.improvements.push(format!(
                    "\u{2705} IMPROVEMENT {identity}.{field}: {frozen:?} -> {cur:?} \u{2014} now matches the in-game reference target; consider re-freezing and graduating"
                ));
            }
        }
        Some(cur) if Some(cur) == frozen => {}
        other => ctx.failures.push(format!(
            "{identity}: {field} drifted to {other:?} (neither frozen {frozen:?} nor reference target {target:?})"
        )),
    }
}

fn rgba_distance(a: [f32; 4], b: [f32; 4]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
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
