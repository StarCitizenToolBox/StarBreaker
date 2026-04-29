"""MCP-friendly animation query and apply tools.

These functions are designed to be called from MCP-driven code (e.g.
``execute_blender_code``) where there is no window-manager context or
active-object selection. They accept a *package-root object name*
(string) instead of requiring the object to be selected in the viewport.

All functions work by looking up the package-root object by name in
``bpy.data.objects``, loading the ``PackageBundle`` from its
``starbreaker_scene_path`` property, and delegating to existing
addon internals.
"""

from __future__ import annotations

import json
import bpy
from typing import TYPE_CHECKING, Any

from starbreaker_addon.manifest import PackageBundle
from starbreaker_addon.runtime.package_ops import (
    _find_animation_selection,
    _clip_bone_hashes,
    _fragment_reverse_playback,
    apply_animation_mode_to_package_root,
    _restore_bind_pose,
    _apply_animation_mode_for_clip,
    _find_animation_clip,
    _ANIMATION_MODES_PROP,
    _parse_fragment_animation_key,
    _FRAGMENT_ANIMATION_PREFIX,
)

if TYPE_CHECKING:
    from bpy.types import Object


# ------------------------------------------------------------------
# Public API
# ------------------------------------------------------------------

def get_animation_list(
    context: bpy.types.Context,
    package_root_name: str,
) -> list[dict[str, Any]]:
    """Return a structured list of all animations for a package root.

    Each entry contains:
      - ``short_name``: fragment name (e.g. ``"Canopy"``) — the key used
        when applying animations.
      - ``long_name``: human-readable label built from fragment + clip
        names (e.g. ``"Canopy — canopy_open"``).
      - ``fragment``: the DBA fragment name (e.g. ``"Canopy"``).
      - ``clip_names``: list of clip names within the fragment
        (e.g. ``["canopy_open"]``).
      - ``clip_fps``: frame rate of the clips (from first clip).
      - ``frame_count``: number of frames (from first clip).
      - ``bone_count``: number of bones tracked by the first clip.
      - ``modes``: list of available modes (always ``["none",
        "snap_first", "snap_last", "action"]``).

    Returns an empty list if the package root is not found or has no
    animations.
    """
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return []

    try:
        bundle = PackageBundle.load(_string_prop(obj, "starbreaker_scene_path"))
    except Exception:
        return []

    clips = _animation_clips(bundle)
    if not clips:
        return []

    result: list[dict[str, Any]] = []
    for clip in clips:
        clip_name = clip.get("name", "?")
        fps = clip.get("fps", 30) or 30
        frame_count = clip.get("frame_count", 0)
        bones = clip.get("bones", [])
        bone_count = len(bones) if isinstance(bones, list) else 0
        fragments = clip.get("fragments", [])
        for frag in fragments:
            frag_name = frag.get("fragment", "?")
            anims = frag.get("animations", [])
            # Find the matching clip in this fragment's animations
            matching_anims = [
                a.get("name", "") for a in anims
                if a.get("name") == clip_name
            ]
            if not matching_anims:
                # The clip appeared at the top level without a fragment
                # wrapper — use fragment name as-is
                matching_anims = [clip_name]
            for anim_name in matching_anims:
                result.append({
                    "short_name": frag_name,
                    "long_name": f"{frag_name} — {anim_name}",
                    "fragment": frag_name,
                    "clip_name": clip_name,
                    "clip_fps": fps,
                    "frame_count": frame_count,
                    "bone_count": bone_count,
                    "modes": ["none", "snap_first", "snap_last", "action"],
                })

    return result


def resolve_animation_key(
    context: bpy.types.Context,
    package_root_name: str,
    animation_name: str,
) -> str | None:
    """Resolve a user-friendly animation name to the internal fragment:key.

    Accepts either:
    - A fragment name (e.g. ``"Canopy"``) — resolves to the first matching
      clip within that fragment.
    - A full fragment:key (e.g. ``"fragment:0:canopy_open"``) — passed
      through unchanged.

    Returns the resolved key string, or ``None`` if not found.
    """
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return None

    try:
        scene_path = _string_prop(obj, "starbreaker_scene_path")
        if scene_path is None:
            return None
        bundle = PackageBundle.load(scene_path)
    except Exception:
        return None

    clips = _animation_clips(bundle)
    if not clips:
        return None

    # If already in fragment:key form, pass through
    if animation_name.startswith(_FRAGMENT_ANIMATION_PREFIX + "0:"):
        return animation_name

    # Search for the animation name (fragment or clip name)
    for clip_idx, clip in enumerate(clips):
        clip_name = clip.get("name", "")
        fragments = clip.get("fragments", [])
        for frag in fragments:
            frag_name = frag.get("fragment", "")
            anims = frag.get("animations", [])
            for anim in anims:
                anim_clip_name = anim.get("name", "")
                if (frag_name == animation_name or
                        anim_clip_name == animation_name or
                        frag_name.lower() == animation_name.lower()):
                    return f"{_FRAGMENT_ANIMATION_PREFIX}{clip_idx}:{anim_clip_name}"

    return None


def apply_animation_mode(
    context: bpy.types.Context,
    package_root_name: str,
    animation_name: str,
    mode: str,
) -> dict[str, Any]:
    """Apply an animation in one of the standard modes.

    Parameters
    ----------
    animation_name : str
        Either a fragment name (e.g. ``"Canopy"``) — resolves to the
        first matching clip within that fragment — or a full
        fragment:key (e.g. ``"fragment:0:canopy_open"``).
    mode : str
        One of ``"none"``, ``"snap_first"``, ``"snap_last"``,
        ``"action"``.

    Returns
    -------
    dict
        ``{"status": "ok"|"error", "updated_count": int, "message": str}``
    """
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return {
            "status": "error",
            "updated_count": 0,
            "message": f"Object '{package_root_name}' not found",
        }

    # Resolve the animation name to internal format
    resolved = resolve_animation_key(context, package_root_name, animation_name)
    if resolved is None:
        return {
            "status": "error",
            "updated_count": 0,
            "message": f"Animation '{animation_name}' not found",
        }

    try:
        count = apply_animation_mode_to_package_root(
            context, obj, resolved, mode,
        )
        return {
            "status": "ok",
            "updated_count": count,
            "message": f"{animation_name} {mode}: {count} object(s) updated",
        }
    except Exception as exc:
        return {
            "status": "error",
            "updated_count": 0,
            "message": str(exc),
        }


def clear_animation_mode(
    context: bpy.types.Context,
    package_root_name: str,
    animation_name: str,
) -> dict[str, Any]:
    """Convenience wrapper for the "None" mode — clears an animation.

    Equivalent to calling ``apply_animation_mode(..., mode="none")``.
    """
    return apply_animation_mode(
        context, package_root_name, animation_name, "none",
    )


# ------------------------------------------------------------------
# Helpers (mirrored from package_ops where we can't import privately)
# ------------------------------------------------------------------

def _string_prop(obj: Object, key: str) -> str | None:
    """Safely read a string custom property from an object."""
    val = obj.get(key)
    return val if isinstance(val, str) else None


def _animation_clips(bundle: PackageBundle) -> list[dict[str, Any]]:
    """Return the list of animation clips from the package's scene root."""
    raw = bundle.scene.root_entity.raw
    clips = raw.get("animations") if isinstance(raw, dict) else None
    if not isinstance(clips, list):
        return []
    result: list[dict[str, Any]] = []
    for clip in clips:
        if isinstance(clip, dict):
            result.append(clip)
    return result