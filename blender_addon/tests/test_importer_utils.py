"""Unit tests for starbreaker_addon.runtime.importer.utils helpers.

Covers the nearest-index duplicate-name selection logic added in Phase 57
(``_remapped_submaterial_for_slot``) and the helper builders
``_submaterials_by_name`` / ``_unique_submaterials_by_name``.

These tests run without a live Blender process (``bpy`` is stubbed out).
"""

from __future__ import annotations

import sys
import types
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))

# ---------------------------------------------------------------------------
# Stub bpy and mathutils so that utils.py can be imported outside Blender.
# ---------------------------------------------------------------------------

if "mathutils" not in sys.modules:
    mathutils = types.ModuleType("mathutils")

    class _Matrix(tuple):
        def __new__(cls, rows):
            return tuple.__new__(cls, rows)

        def inverted(self):
            return self

    class _Quaternion(tuple):
        def __new__(cls, values):
            return tuple.__new__(cls, values)

    mathutils.Matrix = _Matrix
    mathutils.Quaternion = _Quaternion
    sys.modules["mathutils"] = mathutils

if "bpy" not in sys.modules:
    bpy = types.ModuleType("bpy")
    bpy.types = types.SimpleNamespace(
        Context=object,
        Material=object,
        NodeLinks=object,
        Nodes=object,
        Object=object,
        ShaderNodeTexImage=object,
    )
    bpy.data = types.SimpleNamespace(node_groups=[], images=[])
    sys.modules["bpy"] = bpy

# Ensure package hierarchy is registered.
for _pkg in ("starbreaker_addon", "starbreaker_addon.runtime", "starbreaker_addon.runtime.importer"):
    if _pkg not in sys.modules:
        _mod = types.ModuleType(_pkg)
        _parts = _pkg.replace("starbreaker_addon", str(ADDON_ROOT / "starbreaker_addon"))
        _mod.__path__ = [str(ADDON_ROOT / Path(*_pkg.split(".")))]
        sys.modules[_pkg] = _mod

from starbreaker_addon.manifest import MaterialSidecar, SubmaterialRecord
from starbreaker_addon.runtime.importer.utils import (
    _remapped_submaterial_for_slot,
    _submaterials_by_name,
    _unique_submaterials_by_name,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_submaterial(index: int, name: str) -> SubmaterialRecord:
    """Build a minimal SubmaterialRecord stub for use in unit tests."""
    return SubmaterialRecord.from_value({"index": index, "submaterial_name": name})


def _make_sidecar(*entries: tuple[int, str]) -> MaterialSidecar:
    """Build a MaterialSidecar whose submaterials are the given (index, name) pairs."""
    submaterials = [_make_submaterial(idx, name) for idx, name in entries]
    return MaterialSidecar(
        geometry_path=None,
        normalized_export_relative_path=None,
        source_material_path=None,
        palette_contract={},
        submaterials=submaterials,
        raw={},
    )


# ---------------------------------------------------------------------------
# Tests for _submaterials_by_name
# ---------------------------------------------------------------------------

class TestSubmaterialsByName(unittest.TestCase):
    def test_groups_duplicate_names(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"), (2, "paint"))
        grouped = _submaterials_by_name(sidecar)
        self.assertIn("paint", grouped)
        self.assertEqual(len(grouped["paint"]), 2)
        self.assertEqual([r.index for r in grouped["paint"]], [0, 2])

    def test_single_entry_per_unique_name(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"))
        grouped = _submaterials_by_name(sidecar)
        self.assertEqual(len(grouped["paint"]), 1)
        self.assertEqual(len(grouped["glass"]), 1)

    def test_skips_blank_names(self) -> None:
        sidecar = _make_sidecar((0, ""), (1, "  "), (2, "metal"))
        grouped = _submaterials_by_name(sidecar)
        self.assertNotIn("", grouped)
        self.assertNotIn("  ", grouped)
        self.assertIn("metal", grouped)


# ---------------------------------------------------------------------------
# Tests for _unique_submaterials_by_name
# ---------------------------------------------------------------------------

class TestUniqueSubmaterialsByName(unittest.TestCase):
    def test_excludes_duplicate_names(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"), (2, "paint"))
        unique = _unique_submaterials_by_name(sidecar)
        self.assertNotIn("paint", unique)
        self.assertIn("glass", unique)

    def test_includes_unique_names(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"))
        unique = _unique_submaterials_by_name(sidecar)
        self.assertIn("paint", unique)
        self.assertIn("glass", unique)

    def test_returns_first_record_for_unique(self) -> None:
        sidecar = _make_sidecar((3, "metal"))
        unique = _unique_submaterials_by_name(sidecar)
        self.assertEqual(unique["metal"].index, 3)


# ---------------------------------------------------------------------------
# Tests for _remapped_submaterial_for_slot  (Phase 57 fix)
# ---------------------------------------------------------------------------

class TestRemappedSubmaterialForSlot(unittest.TestCase):

    def _slot(
        self,
        source: SubmaterialRecord | None,
        fallback: int,
        by_index: dict[int, SubmaterialRecord],
        by_name: dict[str, SubmaterialRecord],
        by_name_all: dict[str, list[SubmaterialRecord]] | None = None,
    ) -> SubmaterialRecord | None:
        return _remapped_submaterial_for_slot(source, fallback, by_index, by_name, by_name_all)

    def test_unique_name_match_takes_priority_over_index(self) -> None:
        source = _make_submaterial(0, "glass")
        target_0 = _make_submaterial(0, "metal")
        target_1 = _make_submaterial(1, "glass")
        by_index = {0: target_0, 1: target_1}
        by_name = {"glass": target_1}
        result = self._slot(source, 0, by_index, by_name)
        self.assertIs(result, target_1)

    def test_falls_back_to_index_when_name_absent(self) -> None:
        source = _make_submaterial(0, "nonexistent")
        target_5 = _make_submaterial(5, "something_else")
        by_index = {5: target_5}
        by_name: dict[str, SubmaterialRecord] = {}
        result = self._slot(source, 5, by_index, by_name)
        self.assertIs(result, target_5)

    def test_none_source_falls_back_to_index(self) -> None:
        target_2 = _make_submaterial(2, "paint")
        by_index = {2: target_2}
        result = self._slot(None, 2, by_index, {})
        self.assertIs(result, target_2)

    def test_duplicate_names_picks_nearest_by_index(self) -> None:
        """Phase 57 fix: when a name maps to multiple submaterials, pick the one
        whose index is closest to fallback_index."""
        dup_near = _make_submaterial(3, "paint")
        dup_far = _make_submaterial(9, "paint")
        source = _make_submaterial(4, "paint")  # source index doesn't matter; fallback=4
        by_index: dict[int, SubmaterialRecord] = {}
        by_name: dict[str, SubmaterialRecord] = {}   # name absent from unique map (duplicate)
        by_name_all = {"paint": [dup_near, dup_far]}
        result = self._slot(source, 4, by_index, by_name, by_name_all)
        # |3-4| = 1 < |9-4| = 5 → should pick dup_near
        self.assertIs(result, dup_near)

    def test_duplicate_names_picks_nearest_when_far_is_closer(self) -> None:
        dup_near = _make_submaterial(1, "paint")
        dup_far = _make_submaterial(10, "paint")
        source = _make_submaterial(0, "paint")
        by_name_all = {"paint": [dup_near, dup_far]}
        result = self._slot(source, 9, {}, {}, by_name_all)
        # |1-9| = 8 vs |10-9| = 1 → should pick dup_far
        self.assertIs(result, dup_far)

    def test_by_name_all_not_queried_when_unique_match_exists(self) -> None:
        """If by_name has a unique match, by_name_all should be irrelevant."""
        unique = _make_submaterial(2, "paint")
        dup_a = _make_submaterial(0, "paint")
        dup_b = _make_submaterial(5, "paint")
        source = _make_submaterial(0, "paint")
        by_name = {"paint": unique}
        by_name_all = {"paint": [dup_a, dup_b]}
        result = self._slot(source, 0, {}, by_name, by_name_all)
        self.assertIs(result, unique)

    def test_blank_source_name_falls_back_to_index(self) -> None:
        source = _make_submaterial(0, "   ")  # blank name after strip
        target_7 = _make_submaterial(7, "anything")
        by_index = {7: target_7}
        result = self._slot(source, 7, by_index, {})
        self.assertIs(result, target_7)

    def test_returns_none_when_index_also_missing(self) -> None:
        source = _make_submaterial(0, "ghost")
        result = self._slot(source, 99, {}, {})
        self.assertIsNone(result)


if __name__ == "__main__":
    unittest.main()
