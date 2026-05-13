"""MCP-friendly animation query and apply tools.

These functions are designed to be called from MCP-driven code (e.g.
``execute_blender_code``) where there is no window-manager context or
active-object selection. They accept a *package-root object name*
(string) instead of requiring the object to be selected in the viewport.

All functions work by looking up the package-root object by name in
``bpy.data.objects``, loading the package via the same scene-path
resolution as the UI operators, and delegating to existing addon internals.
"""

from __future__ import annotations

import json
import bpy
from typing import TYPE_CHECKING, Any

from starbreaker_addon.runtime.package_ops import (
    _load_package_from_root,
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
    package_animation_instances,
    update_animation_instance_start_frame,
    delete_animation_instance,
    set_animation_instance_muted,
    solo_animation_instance,
)

if TYPE_CHECKING:
    from bpy.types import Object
    from starbreaker_addon.manifest import PackageBundle


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
        bundle = _load_package_from_root(obj)
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
        bundle = _load_package_from_root(obj)
    except Exception:
        return None

    clips = _animation_clips(bundle)
    if not clips:
        return None

    # If already in fragment:key form, pass through.
    if _parse_fragment_animation_key(animation_name) is not None:
        return animation_name

    target = animation_name.strip()
    target_lower = target.lower()

    # Exact clip names are canonical and disambiguate paired fragments such as
    # Scorpius Wings Deploy/Retract.
    for clip in clips:
        clip_name = str(clip.get("name", "")).strip()
        if clip_name and clip_name.lower() == target_lower:
            return clip_name

    # Search fragment labels only after exact clip names have had a chance to
    # resolve. Fragment keys store the fragment index within the clip, not the
    # clip's index in scene.json.
    for clip in clips:
        clip_name = str(clip.get("name", "")).strip()
        fragments = clip.get("fragments", [])
        if not clip_name or not isinstance(fragments, list):
            continue
        for fragment_index, frag in enumerate(fragments):
            if not isinstance(frag, dict):
                continue
            frag_name = str(frag.get("fragment", "")).strip()
            anims = frag.get("animations", [])
            if frag_name.lower() == target_lower:
                return f"{_FRAGMENT_ANIMATION_PREFIX}{fragment_index}:{clip_name}"
            if not isinstance(anims, list):
                continue
            for anim in anims:
                if not isinstance(anim, dict):
                    continue
                anim_clip_name = str(anim.get("name", "")).strip()
                if anim_clip_name and anim_clip_name.lower() == target_lower:
                    return clip_name

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


def list_animation_instances(
    context: bpy.types.Context,
    package_root_name: str,
) -> list[dict[str, Any]]:
    """Return tracked animation instances for the package root.

    This is MCP-friendly and does not require active selection.
    """
    del context
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return []
    try:
        bundle = _load_package_from_root(obj)
        return package_animation_instances(obj, bundle)
    except Exception:
        return []


def set_instance_start_frame(
    context: bpy.types.Context,
    package_root_name: str,
    instance_id: str,
    start_frame: float,
) -> dict[str, Any]:
    """Set one animation instance start frame by id."""
    del context
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return {"status": "error", "message": f"Object '{package_root_name}' not found"}
    try:
        bundle = _load_package_from_root(obj)
        ok = update_animation_instance_start_frame(obj, instance_id, float(start_frame), package=bundle)
        if not ok:
            return {"status": "error", "message": "Animation instance not found"}
        return {"status": "ok", "message": f"Moved to frame {int(round(float(start_frame)))}"}
    except Exception as exc:
        return {"status": "error", "message": str(exc)}


def delete_instance(
    context: bpy.types.Context,
    package_root_name: str,
    instance_id: str,
) -> dict[str, Any]:
    """Delete one animation instance by id."""
    del context
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return {"status": "error", "message": f"Object '{package_root_name}' not found"}
    try:
        bundle = _load_package_from_root(obj)
        ok = delete_animation_instance(obj, instance_id, package=bundle)
        if not ok:
            return {"status": "error", "message": "Animation instance not found"}
        return {"status": "ok", "message": "Deleted animation instance"}
    except Exception as exc:
        return {"status": "error", "message": str(exc)}


def toggle_instance_mute(
    context: bpy.types.Context,
    package_root_name: str,
    instance_id: str,
) -> dict[str, Any]:
    """Toggle mute on one animation instance by id."""
    del context
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return {"status": "error", "message": f"Object '{package_root_name}' not found"}
    try:
        bundle = _load_package_from_root(obj)
        muted = set_animation_instance_muted(obj, instance_id, None, package=bundle)
        if muted is None:
            return {"status": "error", "message": "Animation instance not found"}
        return {
            "status": "ok",
            "message": "Muted animation instance" if muted else "Unmuted animation instance",
            "muted": bool(muted),
        }
    except Exception as exc:
        return {"status": "error", "message": str(exc)}


def solo_instance(
    context: bpy.types.Context,
    package_root_name: str,
    instance_id: str,
) -> dict[str, Any]:
    """Solo one animation instance by muting all other tracked instances."""
    del context
    obj = bpy.data.objects.get(package_root_name)
    if obj is None:
        return {"status": "error", "message": f"Object '{package_root_name}' not found"}
    try:
        bundle = _load_package_from_root(obj)
        ok = solo_animation_instance(obj, instance_id, package=bundle)
        if not ok:
            return {"status": "error", "message": "Animation instance not found"}
        return {"status": "ok", "message": "Soloed animation instance"}
    except Exception as exc:
        return {"status": "error", "message": str(exc)}


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
