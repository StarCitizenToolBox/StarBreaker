use std::fs;
use std::path::Path;

fn load_engine_module_source(module_dir: &str) -> String {
    let module_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(module_dir);
    let mut merged = fs::read_to_string(module_root.join("engine.inc"))
        .unwrap_or_else(|err| panic!("failed to read {module_dir}/engine.inc: {err}"));

    let parts_dir = module_root.join("engine_parts");
    if parts_dir.is_dir() {
        let mut part_paths: Vec<_> = fs::read_dir(&parts_dir)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", parts_dir.display()))
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("part"))
            .collect();
        part_paths.sort();
        for path in part_paths {
            let chunk = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            merged.push('\n');
            merged.push_str(&chunk);
        }
    }

    merged
}

#[test]
fn hardcoding_guard_tests_exist_in_core_renderer_files() {
    let guarded_files: Vec<(&str, String, &str)> = vec![
        (
            "compose/tests.rs",
            include_str!("../src/compose/tests.rs").to_string(),
            "fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "ir_compose.rs",
            load_engine_module_source("ir_compose"),
            "fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "ui_ir.rs",
            load_engine_module_source("ui_ir"),
            "fn ui_ir_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "bb_layout.rs",
            load_engine_module_source("bb_layout"),
            "fn layout_source_does_not_reintroduce_forbidden_hardcoded_or_heuristic_markers()",
        ),
    ];

    for (path, source, guard_fn_sig) in guarded_files {
        assert!(
            source.contains(guard_fn_sig),
            "required hardcoding guard test missing in {path}: expected `{guard_fn_sig}`",
        );
    }
}

#[test]
fn bb_layout_source_has_no_forbidden_heuristic_markers() {
    let source = load_engine_module_source("bb_layout");
    let forbidden = [
        ["hard", "coded", "_offset"].concat(),
        ["magic", "_multiplier"].concat(),
        ["heu", "ristic", "_shift"].concat(),
        ["blend", "_factor"].concat(),
    ];

    for marker in forbidden {
        assert!(
            !source.contains(marker.as_str()),
            "bb_layout heuristic marker reintroduced: {marker}",
        );
    }
}
