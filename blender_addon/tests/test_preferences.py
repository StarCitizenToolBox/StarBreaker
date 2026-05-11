"""Phase 63 — Addon preference gate unit tests.

Tests for the pure helper functions that control post-import behaviour.
These tests do not require bpy and run in a plain Python environment.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))


def _load_preference_gates() -> tuple:
    """Extract pure preference gate functions from ui.py via AST, no bpy needed."""
    import ast as ast_mod

    ui_path = ADDON_ROOT / "starbreaker_addon" / "ui.py"
    source = ui_path.read_text()
    tree = ast_mod.parse(source)

    namespace: dict = {}
    for node in ast_mod.walk(tree):
        if isinstance(node, ast_mod.FunctionDef) and node.name in (
            "_should_apply_landing_gear",
            "_should_change_viewport",
        ):
            func_source = ast_mod.get_source_segment(source, node)
            if func_source:
                exec(compile(ast_mod.parse(func_source), str(ui_path), "exec"), namespace)  # noqa: S102

    return (
        namespace["_should_apply_landing_gear"],
        namespace["_should_change_viewport"],
    )


(
    _should_apply_landing_gear,
    _should_change_viewport,
) = _load_preference_gates()


class _MockPrefs:
    """Simple stand-in for STARBREAKER_AP_preferences."""

    def __init__(
        self,
        landing_gear_retract_after_import: bool = True,
        viewport_change_after_import: bool = True,
    ) -> None:
        self.landing_gear_retract_after_import = landing_gear_retract_after_import
        self.viewport_change_after_import = viewport_change_after_import


class TestPreferenceGates(unittest.TestCase):
    # ── _should_apply_landing_gear ───────────────────────────────────────────

    def test_landing_gear_default_true_when_prefs_none(self) -> None:
        self.assertTrue(_should_apply_landing_gear(None))

    def test_landing_gear_true_when_pref_enabled(self) -> None:
        self.assertTrue(_should_apply_landing_gear(_MockPrefs(landing_gear_retract_after_import=True)))

    def test_landing_gear_false_when_pref_disabled(self) -> None:
        self.assertFalse(_should_apply_landing_gear(_MockPrefs(landing_gear_retract_after_import=False)))

    def test_landing_gear_true_when_attr_missing(self) -> None:
        """Gracefully defaults to True when the attribute is absent."""
        self.assertTrue(_should_apply_landing_gear(object()))

    # ── _should_change_viewport ──────────────────────────────────────────────

    def test_viewport_default_true_when_prefs_none(self) -> None:
        self.assertTrue(_should_change_viewport(None))

    def test_viewport_true_when_pref_enabled(self) -> None:
        self.assertTrue(_should_change_viewport(_MockPrefs(viewport_change_after_import=True)))

    def test_viewport_false_when_pref_disabled(self) -> None:
        self.assertFalse(_should_change_viewport(_MockPrefs(viewport_change_after_import=False)))

    def test_viewport_true_when_attr_missing(self) -> None:
        """Gracefully defaults to True when the attribute is absent."""
        self.assertTrue(_should_change_viewport(object()))

    # ── Independence ─────────────────────────────────────────────────────────

    def test_each_pref_is_independent(self) -> None:
        """Disabling one pref must not affect the other."""
        prefs_no_gear = _MockPrefs(landing_gear_retract_after_import=False, viewport_change_after_import=True)
        self.assertFalse(_should_apply_landing_gear(prefs_no_gear))
        self.assertTrue(_should_change_viewport(prefs_no_gear))

        prefs_no_view = _MockPrefs(landing_gear_retract_after_import=True, viewport_change_after_import=False)
        self.assertTrue(_should_apply_landing_gear(prefs_no_view))
        self.assertFalse(_should_change_viewport(prefs_no_view))

    def test_both_disabled(self) -> None:
        prefs = _MockPrefs(landing_gear_retract_after_import=False, viewport_change_after_import=False)
        self.assertFalse(_should_apply_landing_gear(prefs))
        self.assertFalse(_should_change_viewport(prefs))


if __name__ == "__main__":
    unittest.main()
