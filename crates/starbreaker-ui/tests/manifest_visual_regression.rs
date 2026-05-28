use std::path::PathBuf;
use starbreaker_ui::{
    UiIrDocument, UiRegressionManifest, UiScreenSnapshot, compare_manifest_targets_with_loader,
    snapshot_from_ui_ir,
};
use std::collections::HashMap;

fn artifact_paths(target_id: &str) -> (PathBuf, PathBuf) {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should resolve from CARGO_MANIFEST_DIR");
    let workspace_root = repo_root
        .parent()
        .expect("repo root should have workspace parent");
    let manifest_json: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/ui_ir/ui_snapshot_manifest.json"
    ))
    .expect("manifest JSON fixture should parse");
    let source_png = manifest_json
        .get("targets")
        .and_then(|targets| targets.as_array())
        .and_then(|targets| {
            targets.iter().find_map(|target| {
                if target.get("id").and_then(|id| id.as_str()) == Some(target_id) {
                    target
                        .get("source_generated_png")
                        .and_then(|value| value.as_str())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| {
            panic!("target {target_id} missing source_generated_png in regression manifest")
        });

    let source_path = if source_png.starts_with('/') {
        PathBuf::from(source_png)
    } else if source_png.starts_with("ships/") {
        workspace_root.join(source_png)
    } else {
        repo_root.join(source_png)
    };
    let artifact_path = repo_root.join("test-artifacts/ui").join(format!("{target_id}.png"));
    (source_path, artifact_path)
}

fn snapshot_manifest() -> UiRegressionManifest {
    serde_json::from_str(include_str!("fixtures/ui_ir/ui_snapshot_manifest.json"))
    .expect("snapshot manifest fixture should parse")
}

fn manifest_snapshot_lookup() -> HashMap<String, UiScreenSnapshot> {
    let ui_target_a: UiIrDocument = serde_json::from_str(include_str!(
        "fixtures/ui_ir/target_a-screen_16x9_a-ir.json"
    ))
    .expect("ui_target_a IR fixture should parse");
    let ui_target_b: UiIrDocument = serde_json::from_str(include_str!(
        "fixtures/ui_ir/target_b-mesh_end_screen_plane-ir.json"
    ))
    .expect("ui_target_b IR fixture should parse");

    HashMap::from([
        ("ui_target_a.baseline".to_string(), snapshot_from_ui_ir(&ui_target_a)),
        ("ui_target_a.current".to_string(), snapshot_from_ui_ir(&ui_target_a)),
        ("ui_target_b.baseline".to_string(), snapshot_from_ui_ir(&ui_target_b)),
        ("ui_target_b.current".to_string(), snapshot_from_ui_ir(&ui_target_b)),
        (
            "clipper_small_door.baseline".to_string(),
            snapshot_from_ui_ir(&ui_target_b),
        ),
        (
            "clipper_small_door.current".to_string(),
            snapshot_from_ui_ir(&ui_target_b),
        ),
    ])
}

fn assert_manifest_runner_preflight() {
    let manifest = snapshot_manifest();
    let snapshots = manifest_snapshot_lookup();
    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot fixture for {path}"))
    })
    .expect("manifest runner should load all manifest fixture snapshots");
    for result in results {
        assert!(
            result.comparison.passed,
            "manifest snapshot preflight failed for {}: {:?}",
            result.id,
            result.comparison.failures
        );
    }
}

fn cyan_text_coverage(img: &image::RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) -> f32 {
    let mut hits = 0u64;
    let mut total = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = img.get_pixel(x, y);
            total += 1;
            if px[1] > 140 && px[2] > 140 && px[0] < 170 {
                hits += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        hits as f32 / total as f32
    }
}

fn foreground_mask_from_border_delta(
    img: &image::RgbaImage,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Option<(Vec<bool>, usize, usize)> {
    let (img_w, img_h) = img.dimensions();
    let x0 = x.round().max(0.0) as u32;
    let y0 = y.round().max(0.0) as u32;
    let x1 = (x + w).round().max(0.0) as u32;
    let y1 = (y + h).round().max(0.0) as u32;

    let x0 = x0.min(img_w);
    let y0 = y0.min(img_h);
    let x1 = x1.min(img_w);
    let y1 = y1.min(img_h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let width = (x1 - x0) as usize;
    let height = (y1 - y0) as usize;

    let mut border_r = Vec::new();
    let mut border_g = Vec::new();
    let mut border_b = Vec::new();

    for x in x0..x1 {
        let top = img.get_pixel(x, y0);
        let bottom = img.get_pixel(x, y1 - 1);
        border_r.push(top[0]);
        border_g.push(top[1]);
        border_b.push(top[2]);
        border_r.push(bottom[0]);
        border_g.push(bottom[1]);
        border_b.push(bottom[2]);
    }
    for y in y0..y1 {
        let left = img.get_pixel(x0, y);
        let right = img.get_pixel(x1 - 1, y);
        border_r.push(left[0]);
        border_g.push(left[1]);
        border_b.push(left[2]);
        border_r.push(right[0]);
        border_g.push(right[1]);
        border_b.push(right[2]);
    }

    if border_r.is_empty() {
        return None;
    }

    border_r.sort_unstable();
    border_g.sort_unstable();
    border_b.sort_unstable();
    let mid = border_r.len() / 2;
    let bg_r = border_r[mid] as i32;
    let bg_g = border_g[mid] as i32;
    let bg_b = border_b[mid] as i32;

    let mut mask = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let px = img.get_pixel(x0 + x as u32, y0 + y as u32);
            let delta = (px[0] as i32 - bg_r).abs()
                + (px[1] as i32 - bg_g).abs()
                + (px[2] as i32 - bg_b).abs();
            mask[y * width + x] = delta > 30;
        }
    }

    Some((mask, width, height))
}

fn mask_touches_all_edges(mask: &[bool], width: usize, height: usize, band: usize) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let top_band = band.min(height);
    let left_band = band.min(width);

    let touches_top = (0..top_band).any(|y| (0..width).any(|x| mask[y * width + x]));
    let touches_bottom = (height - top_band..height).any(|y| (0..width).any(|x| mask[y * width + x]));
    let touches_left = (0..height).any(|y| (0..left_band).any(|x| mask[y * width + x]));
    let touches_right = (0..height)
        .any(|y| (width - left_band..width).any(|x| mask[y * width + x]));

    touches_top && touches_bottom && touches_left && touches_right
}

fn assert_manifest_visual_regression_guard(
    name: &str,
    min_allowed_coverage_ratio: f32,
    max_allowed_coverage_ratio: f32,
) {
    let (reference_path, current_path) = artifact_paths(name);
    if !reference_path.is_file() || !current_path.is_file() {
        let require_artifacts = std::env::var("STARBREAKER_UI_REQUIRE_VISUAL_ARTIFACTS")
            .map(|value| value == "1")
            .unwrap_or(false);
        if require_artifacts {
            panic!(
                "missing required visual regression artifacts for {name}: reference={} current={}",
                reference_path.display(),
                current_path.display()
            );
        }
        eprintln!(
            "skipping {name} visual regression guard (missing files: reference={} current={})",
            reference_path.display(),
            current_path.display()
        );
        return;
    }

    let reference = image::open(&reference_path)
        .expect("reference image should decode")
        .into_rgba8();
    let current = image::open(&current_path)
        .expect("current image should decode")
        .into_rgba8();
    assert_eq!(
        reference.dimensions(),
        current.dimensions(),
        "{name} dimensions drifted: reference={} current={}",
        reference_path.display(),
        current_path.display()
    );

    let (width, height) = reference.dimensions();
    let x0 = (width as f32 * 0.20) as u32;
    let x1 = (width as f32 * 0.80) as u32;
    let y0 = (height as f32 * 0.20) as u32;
    let y1 = (height as f32 * 0.60) as u32;

    assert_roi_coverage_ratio(
        &reference,
        &current,
        x0,
        y0,
        x1,
        y1,
        name,
        "central text ROI",
        min_allowed_coverage_ratio,
        max_allowed_coverage_ratio,
        &reference_path,
        &current_path,
    );
}

fn assert_roi_coverage_ratio(
    reference: &image::RgbaImage,
    current: &image::RgbaImage,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    name: &str,
    roi_name: &str,
    min_allowed_coverage_ratio: f32,
    max_allowed_coverage_ratio: f32,
    reference_path: &std::path::Path,
    current_path: &std::path::Path,
) {
    let reference_coverage = cyan_text_coverage(reference, x0, y0, x1, y1);
    let current_coverage = cyan_text_coverage(&current, x0, y0, x1, y1);
    let coverage_ratio = current_coverage / reference_coverage.max(1e-6);

    assert!(
        coverage_ratio >= min_allowed_coverage_ratio,
        "{name} font-size regression detected: cyan text coverage too low in {roi_name} (ratio {coverage_ratio:.3} < {min_allowed_coverage_ratio:.3}).\nreference={}\ncurrent={}",
        reference_path.display(),
        current_path.display()
    );
    assert!(
        coverage_ratio <= max_allowed_coverage_ratio,
        "{name} font-size regression detected: cyan text coverage too high in {roi_name} (ratio {coverage_ratio:.3} > {max_allowed_coverage_ratio:.3}).\nreference={}\ncurrent={}",
        reference_path.display(),
        current_path.display()
    );
}

#[test]
fn target_a_visual_regression_guard() {
    assert_manifest_runner_preflight();
    // ui_target_a has more animated cyan geometry in the central ROI than ui_target_b,
    // so we use a slightly wider lower bound to avoid false font-size failures.
    assert_manifest_visual_regression_guard("ui_target_a", 0.55, 1.25);
}

#[test]
fn target_b_visual_regression_guard() {
    assert_manifest_runner_preflight();
    assert_manifest_visual_regression_guard("ui_target_b", 0.75, 1.25);
}

#[test]
fn target_a_custom_shape_scale_and_position_guard() {
    let (reference_path, current_path) = artifact_paths("ui_target_a");
    if !reference_path.is_file() || !current_path.is_file() {
        eprintln!(
            "skipping ui_target_a custom-shape guard (missing files: reference={} current={})",
            reference_path.display(),
            current_path.display()
        );
        return;
    }

    let fixture: UiIrDocument = serde_json::from_str(include_str!(
        "fixtures/ui_ir/target_a-screen_16x9_a-ir.json"
    ))
    .expect("ui_target_a IR fixture should parse");
    let mut custom_shape_rects: Vec<(u32, f32, f32, f32, f32)> = fixture
        .nodes
        .iter()
        .filter(|node| node.node_type == "widget_custom_shape" && node.asset_ref.is_some())
        .map(|node| {
            (
                node.id,
                node.computed_rect.x,
                node.computed_rect.y,
                node.computed_rect.w,
                node.computed_rect.h,
            )
        })
        .collect();
    custom_shape_rects.sort_by_key(|entry| entry.0);
    assert!(
        !custom_shape_rects.is_empty(),
        "expected at least one asset-backed custom shape in ui_target_a fixture"
    );

    let reference = image::open(&reference_path)
        .expect("reference image should decode")
        .into_rgba8();
    let current = image::open(&current_path)
        .expect("current image should decode")
        .into_rgba8();

    for (node_id, x, y, w, h) in custom_shape_rects {
        let (reference_mask, width, height) = foreground_mask_from_border_delta(&reference, x, y, w, h)
            .expect("reference mask should be available");
        let (current_mask, _, _) = foreground_mask_from_border_delta(&current, x, y, w, h)
            .expect("current mask should be available");

        let reference_edge_anchored = mask_touches_all_edges(&reference_mask, width, height, 3);
        let current_edge_anchored = mask_touches_all_edges(&current_mask, width, height, 3);
        assert!(
            reference_edge_anchored == current_edge_anchored,
            "ui_target_a custom-shape scale/position drift for node {node_id}: edge anchoring changed between source and artifact"
        );
    }
}
