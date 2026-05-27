use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use starbreaker_ui::defaults::DefaultValueRegistry;
use starbreaker_ui::ui_ir::compile_ui_ir_from_scene;
use starbreaker_ui::ui_snapshot::{
    compare_snapshots, snapshot_from_ui_ir, UiScreenSnapshot, UiSnapshotTolerance,
};

#[derive(Debug, Deserialize)]
struct CertificationSet {
    version: u32,
    cases: Vec<CertificationCase>,
}

#[derive(Debug, Deserialize)]
struct CertificationCase {
    family: String,
    id: String,
    canvas_fixture: String,
    target_width: u32,
    target_height: u32,
}

#[derive(Debug, Serialize)]
struct CertificationCaseResult {
    family: String,
    id: String,
    status: String,
    failures: Vec<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let update_baseline = args.iter().any(|arg| arg == "--update-baseline");

    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_root
        .parent()
        .and_then(Path::parent)
        .ok_or("failed to resolve repository root")?
        .to_path_buf();
    let workspace_root = repo_root
        .parent()
        .ok_or("failed to resolve workspace root")?
        .to_path_buf();

    let config_path = crate_root.join("data/phase5_certification_set_v1.json");
    let config: CertificationSet = serde_json::from_slice(&fs::read(&config_path)?)?;

    let fixture_dir = crate_root.join("tests/fixtures/canvas");
    let baseline_dir = crate_root.join("tests/fixtures/phase5/baseline_snapshots");
    let current_dir = crate_root.join("tests/fixtures/phase5/current_snapshots");
    let artifact_dir = workspace_root.join("docs/StarBreaker/ui-rework-artifacts/phase-5");

    fs::create_dir_all(&baseline_dir)?;
    fs::create_dir_all(&current_dir)?;
    fs::create_dir_all(&artifact_dir)?;

    let defaults = DefaultValueRegistry::with_pipeline_defaults(None);
    let tolerance = UiSnapshotTolerance::default();

    let mut family_totals: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut case_results: Vec<CertificationCaseResult> = Vec::new();

    for case in &config.cases {
        let fixture_path = fixture_dir.join(&case.canvas_fixture);
        let canvas_json: serde_json::Value = serde_json::from_slice(&fs::read(&fixture_path)?)?;
        let scene = starbreaker_ui::bb_scene::parse_bb_canvas(&canvas_json)?;
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            &case.id,
            None,
            (case.target_width, case.target_height),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );
        let snapshot = snapshot_from_ui_ir(&ir);

        let current_snapshot_path = current_dir.join(format!("{}-snapshot.json", case.id));
        fs::write(
            &current_snapshot_path,
            serde_json::to_vec_pretty(&snapshot)?,
        )?;

        let baseline_snapshot_path = baseline_dir.join(format!("{}-snapshot.json", case.id));
        let result = if update_baseline || !baseline_snapshot_path.exists() {
            fs::write(
                &baseline_snapshot_path,
                serde_json::to_vec_pretty(&snapshot)?,
            )?;
            CertificationCaseResult {
                family: case.family.clone(),
                id: case.id.clone(),
                status: "bootstrapped".to_string(),
                failures: Vec::new(),
            }
        } else {
            let baseline: UiScreenSnapshot =
                serde_json::from_slice(&fs::read(&baseline_snapshot_path)?)?;
            let comparison = compare_snapshots(&baseline, &snapshot, tolerance);
            CertificationCaseResult {
                family: case.family.clone(),
                id: case.id.clone(),
                status: if comparison.passed {
                    "pass".to_string()
                } else {
                    "fail".to_string()
                },
                failures: comparison.failures,
            }
        };

        let entry = family_totals.entry(case.family.clone()).or_insert((0, 0));
        entry.0 += 1;
        if result.status == "pass" || result.status == "bootstrapped" {
            entry.1 += 1;
        }
        case_results.push(result);
    }

    let dashboard_path = artifact_dir.join("certification-dashboard.md");
    fs::write(
        &dashboard_path,
        build_dashboard_markdown(&config, &case_results, &family_totals),
    )?;

    let json_report_path = artifact_dir.join("certification-results.json");
    fs::write(&json_report_path, serde_json::to_vec_pretty(&case_results)?)?;

    println!("wrote {}", dashboard_path.display());
    println!("wrote {}", json_report_path.display());

    Ok(())
}

fn build_dashboard_markdown(
    config: &CertificationSet,
    case_results: &[CertificationCaseResult],
    family_totals: &BTreeMap<String, (u32, u32)>,
) -> String {
    let mut out = String::new();
    out.push_str("# Phase 5 Certification Dashboard\n\n");
    out.push_str(&format!("Certification set version: {}\n\n", config.version));

    out.push_str("## Family Summary\n\n");
    out.push_str("| Family | Passed | Total |\n");
    out.push_str("| --- | ---: | ---: |\n");
    for (family, (total, passed)) in family_totals {
        out.push_str(&format!("| {} | {} | {} |\n", family, passed, total));
    }

    out.push_str("\n## Case Results\n\n");
    out.push_str("| Family | Case | Status | Failure Count |\n");
    out.push_str("| --- | --- | --- | ---: |\n");
    for result in case_results {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            result.family,
            result.id,
            result.status,
            result.failures.len()
        ));
    }

    out.push_str("\n## Failures\n\n");
    for result in case_results {
        if result.failures.is_empty() {
            continue;
        }
        out.push_str(&format!("### {} ({})\n\n", result.id, result.family));
        for failure in &result.failures {
            out.push_str(&format!("- {}\n", failure));
        }
        out.push('\n');
    }

    if !case_results.iter().any(|result| !result.failures.is_empty()) {
        out.push_str("No failures reported.\n");
    }

    out
}
