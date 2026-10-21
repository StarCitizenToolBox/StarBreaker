"""Tests for animation FPS reconciliation helpers (Phase 28)."""

from __future__ import annotations

import sys
import types
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))

for _pkg in ("starbreaker_addon", "starbreaker_addon.runtime"):
    if _pkg not in sys.modules:
        _mod = types.ModuleType(_pkg)
        _mod.__path__ = [str(ADDON_ROOT / Path(*_pkg.split(".")))]
        sys.modules[_pkg] = _mod

from starbreaker_addon.runtime.animation_fps import (
    FPS_POLICY_ADAPT_SCENE,
    FPS_POLICY_MATCH_SCENE_TO_CLIP,
    extend_frame_end,
    reconcile_animation_fps,
)


class TestExtendFrameEnd(unittest.TestCase):
    def test_extends_to_longest_strip(self) -> None:
        # Hangar door/elevator clips run far past the default 250-frame range.
        self.assertEqual(extend_frame_end(250, [648.0, 120.0]), 648)

    def test_never_shrinks(self) -> None:
        self.assertEqual(extend_frame_end(1000, [648.0]), 1000)

    def test_no_strips_leaves_range_unchanged(self) -> None:
        self.assertEqual(extend_frame_end(250, []), 250)

    def test_rounds_up_to_whole_frame(self) -> None:
        self.assertEqual(extend_frame_end(250, [647.6]), 648)


class _Render:
    def __init__(self, fps: int, fps_base: float = 1.0) -> None:
        self.fps = fps
        self.fps_base = fps_base


class _Scene:
    def __init__(self, fps: int, fps_base: float = 1.0) -> None:
        self.render = _Render(fps, fps_base)


class TestAnimationFpsReconciliation(unittest.TestCase):
    def test_one_to_one_ratio(self) -> None:
        scene = _Scene(30)
        result = reconcile_animation_fps(scene, 30.0, FPS_POLICY_ADAPT_SCENE)
        self.assertAlmostEqual(result.frame_scale, 1.0)
        self.assertFalse(result.changed_scene_fps)

    def test_two_to_one_ratio_scene_60_clip_30(self) -> None:
        scene = _Scene(60)
        result = reconcile_animation_fps(scene, 30.0, FPS_POLICY_ADAPT_SCENE)
        self.assertAlmostEqual(result.frame_scale, 2.0)
        self.assertFalse(result.changed_scene_fps)

    def test_one_to_two_ratio_scene_30_clip_60(self) -> None:
        scene = _Scene(30)
        result = reconcile_animation_fps(scene, 60.0, FPS_POLICY_ADAPT_SCENE)
        self.assertAlmostEqual(result.frame_scale, 0.5)
        self.assertFalse(result.changed_scene_fps)

    def test_24_to_60_ratio(self) -> None:
        scene = _Scene(60)
        result = reconcile_animation_fps(scene, 24.0, FPS_POLICY_ADAPT_SCENE)
        self.assertAlmostEqual(result.frame_scale, 2.5)
        self.assertFalse(result.changed_scene_fps)

    def test_match_scene_to_clip_updates_scene_fps(self) -> None:
        scene = _Scene(60)
        result = reconcile_animation_fps(scene, 30.0, FPS_POLICY_MATCH_SCENE_TO_CLIP)
        self.assertTrue(result.changed_scene_fps)
        self.assertAlmostEqual(result.scene_fps_after, 30.0, places=5)
        self.assertAlmostEqual(result.frame_scale, 1.0, places=5)


if __name__ == "__main__":
    unittest.main()
