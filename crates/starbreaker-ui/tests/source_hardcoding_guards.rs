#[test]
fn hardcoding_guard_tests_exist_in_core_renderer_files() {
    let guarded_files = [
        (
            "compose/tests.rs",
            include_str!("../src/compose/tests.rs"),
            "fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "ir_compose.rs",
            include_str!("../src/ir_compose.rs"),
            "fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "ui_ir.rs",
            include_str!("../src/ui_ir.rs"),
            "fn ui_ir_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "bb_layout.rs",
            include_str!("../src/bb_layout.rs"),
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
    let source = include_str!("../src/bb_layout.rs");
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
