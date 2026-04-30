from __future__ import annotations

from dataclasses import dataclass
from typing import Any

FPS_POLICY_ADAPT_SCENE = "adapt_scene"
FPS_POLICY_MATCH_SCENE_TO_CLIP = "match_scene_to_clip"
FPS_POLICIES = {FPS_POLICY_ADAPT_SCENE, FPS_POLICY_MATCH_SCENE_TO_CLIP}


@dataclass(frozen=True)
class FpsReconciliation:
    policy: str
    clip_fps: float
    scene_fps_before: float
    scene_fps_after: float
    frame_scale: float
    mismatch: bool
    changed_scene_fps: bool


def _positive_float(value: Any, default: float) -> float:
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return default
    if parsed <= 0.0:
        return default
    return parsed


def _scene_fps(scene: Any) -> float:
    render = getattr(scene, "render", None)
    if render is None:
        return 24.0
    fps = _positive_float(getattr(render, "fps", 24.0), 24.0)
    fps_base = _positive_float(getattr(render, "fps_base", 1.0), 1.0)
    return fps / fps_base


def _normalized_policy(policy: str | None) -> str:
    normalized = (policy or "").strip().lower()
    if normalized in FPS_POLICIES:
        return normalized
    return FPS_POLICY_ADAPT_SCENE


def _apply_scene_fps(scene: Any, fps: float) -> float:
    render = getattr(scene, "render", None)
    if render is None:
        return _scene_fps(scene)
    rounded_fps = max(1, int(round(fps)))
    fps_base = max(rounded_fps / max(fps, 1e-6), 1e-6)
    try:
        render.fps = rounded_fps
        render.fps_base = fps_base
    except Exception:
        return _scene_fps(scene)
    return _scene_fps(scene)


def reconcile_animation_fps(scene: Any, clip_fps_raw: Any, policy: str | None) -> FpsReconciliation:
    normalized_policy = _normalized_policy(policy)
    scene_fps_before = _scene_fps(scene)
    clip_fps = _positive_float(clip_fps_raw, scene_fps_before)
    scene_fps_after = scene_fps_before
    changed_scene_fps = False

    mismatch = abs(scene_fps_before - clip_fps) > 1e-6
    if normalized_policy == FPS_POLICY_MATCH_SCENE_TO_CLIP and mismatch:
        scene_fps_after = _apply_scene_fps(scene, clip_fps)
        changed_scene_fps = abs(scene_fps_after - scene_fps_before) > 1e-6
    frame_scale = scene_fps_after / max(clip_fps, 1e-6)

    return FpsReconciliation(
        policy=normalized_policy,
        clip_fps=clip_fps,
        scene_fps_before=scene_fps_before,
        scene_fps_after=scene_fps_after,
        frame_scale=frame_scale,
        mismatch=mismatch,
        changed_scene_fps=changed_scene_fps,
    )


def describe_reconciliation(clip_name: str, result: FpsReconciliation) -> str:
    if result.policy == FPS_POLICY_MATCH_SCENE_TO_CLIP:
        action = "matched scene fps to clip"
    else:
        action = "adapted clip timing to scene fps"
    return (
        f"[StarBreaker][animation] {clip_name}: clip_fps={result.clip_fps:.3f}, "
        f"scene_fps={result.scene_fps_before:.3f}->{result.scene_fps_after:.3f}, "
        f"policy={result.policy}, frame_scale={result.frame_scale:.4f} ({action})"
    )
