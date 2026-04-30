from __future__ import annotations

import json
from pathlib import Path
import time

import bpy
import blf
import gpu
import mathutils
from bpy.props import BoolProperty, EnumProperty, FloatProperty, StringProperty
from bpy.types import Operator, Panel
from bpy_extras.io_utils import ImportHelper
from gpu_extras.batch import batch_for_shader

from .manifest import PackageBundle
from .palette import resolved_palette_id
from .palette import paint_list_canonical_id
from .runtime import (
    POM_DETAIL_DEFAULT,
    POM_DETAIL_ITEMS,
    PROP_ENTITY_NAME,
    PROP_MATERIAL_SIDECAR,
    PROP_PACKAGE_NAME,
    PROP_PALETTE_ID,
    PROP_SCENE_PATH,
    PROP_SHADER_FAMILY,
    PROP_SURFACE_SHADER_MODE,
    PROP_TEMPLATE_KEY,
    SCENE_POM_DETAIL_PROP,
    TEMPLATE_COLLECTION_NAME,
    apply_pom_detail_mode,
    SCENE_WEAR_STRENGTH_PROP,
    apply_animation_mode_to_package_root,
    apply_light_state,
    apply_livery_to_selected_package,
    apply_paint_to_selected_package,
    apply_palette_to_selected_package,
    available_package_animation_items,
    package_animation_diagnostics,
    available_light_state_names,
    dump_selected_metadata,
    exterior_palette_ids,
    find_package_root,
    import_package,
    package_animation_mode_map,
)


_PAINT_ITEMS_CACHE: list[tuple[str, str, str]] = []
_PALETTE_ITEMS_CACHE: list[tuple[str, str, str]] = []
_LIVERY_ITEMS_CACHE: list[tuple[str, str, str]] = []
_ANIMATION_MODE_ITEMS: tuple[tuple[str, str, str], ...] = (
    ("none", "None", "Leave current transforms (restore bind pose if available)"),
    ("snap_first", "First", "Apply first keyframe pose"),
    ("snap_last", "Last", "Apply last keyframe pose"),
    ("action", "Insert", "Insert full keyframes as Blender Action"),
)
_ANIMATION_FPS_POLICY_ITEMS: tuple[tuple[str, str, str], ...] = (
    (
        "adapt_scene",
        "Adapt To Scene FPS",
        "Keep scene FPS; scale keyframe times by scene_fps/clip_fps",
    ),
    (
        "match_scene_to_clip",
        "Match Scene To Clip FPS",
        "Set scene FPS to clip FPS before inserting keys",
    ),
)
_SCENE_ANIMATION_FPS_POLICY_PROP = "starbreaker_animation_fps_policy"
_IMPORT_PROGRESS_ACTIVE_PROP = "starbreaker_import_progress_active"
_IMPORT_PROGRESS_VALUE_PROP = "starbreaker_import_progress_value"
_IMPORT_PROGRESS_DESCRIPTION_PROP = "starbreaker_import_progress_description"
_IMPORT_PROGRESS_LAST_UPDATE = 0.0
_IMPORT_PROGRESS_DRAW_HANDLER = None


def _progress_fraction(value: float) -> float:
    return max(0.0, min(1.0, float(value)))


def _tag_view3d_redraws(context: bpy.types.Context) -> None:
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if screen is None:
        return
    for area in screen.areas:
        if area.type == "VIEW_3D":
            area.tag_redraw()


def _force_view3d_perspective(context: bpy.types.Context) -> None:
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if window is None or screen is None:
        return
    for area in screen.areas:
        if area.type != "VIEW_3D":
            continue
        region = next((region for region in area.regions if region.type == "WINDOW"), None)
        if region is None:
            continue
        space = getattr(area, "spaces", None)
        active_space = space.active if space is not None else None
        region_3d = getattr(active_space, "region_3d", None)
        if region_3d is None:
            continue
        with context.temp_override(window=window, area=area, region=region):
            region_3d.view_perspective = "PERSP"


def _set_active_object(context: bpy.types.Context, obj: bpy.types.Object | None) -> None:
    view_layer = getattr(context, "view_layer", None)
    if view_layer is not None and obj is not None:
        view_layer.objects.active = obj


def _collapse_template_cache_outliner(context: bpy.types.Context) -> None:
    if bpy.data.collections.get(TEMPLATE_COLLECTION_NAME) is None:
        return
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if window is None or screen is None:
        return
    for area in screen.areas:
        if area.type != "OUTLINER":
            continue
        region = next((region for region in area.regions if region.type == "WINDOW"), None)
        if region is None:
            continue
        with context.temp_override(window=window, area=area, region=region):
            for _ in range(8):
                try:
                    bpy.ops.outliner.show_one_level(open=False)
                except RuntimeError:
                    break


def _frame_package_root_three_quarter(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
) -> None:
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if window is None or screen is None:
        return

    mesh_candidates: list[tuple[bpy.types.Object, float]] = []
    for obj in package_root.children_recursive:
        if obj.type != "MESH":
            continue
        if not bool(getattr(getattr(obj, "data", None), "polygons", ())):
            continue
        if bool(getattr(obj, "hide_viewport", False)):
            continue
        if not obj.visible_get(view_layer=context.view_layer):
            continue
        corners = [obj.matrix_world @ mathutils.Vector(corner) for corner in obj.bound_box]
        if not corners:
            continue
        min_corner = mathutils.Vector(
            (
                min(corner.x for corner in corners),
                min(corner.y for corner in corners),
                min(corner.z for corner in corners),
            )
        )
        max_corner = mathutils.Vector(
            (
                max(corner.x for corner in corners),
                max(corner.y for corner in corners),
                max(corner.z for corner in corners),
            )
        )
        diagonal = (max_corner - min_corner).length
        if diagonal > 0.0:
            mesh_candidates.append((obj, diagonal))

    if not mesh_candidates:
        return

    largest_diagonal = max(diagonal for _, diagonal in mesh_candidates)
    cutoff = largest_diagonal * 0.08
    focus_objects = [obj for obj, diagonal in mesh_candidates if diagonal >= cutoff]
    if not focus_objects:
        focus_objects = [obj for obj, _ in mesh_candidates]

    world_corners: list[mathutils.Vector] = []
    for obj in focus_objects:
        bound_box = getattr(obj, "bound_box", None)
        if not bound_box:
            continue
        matrix_world = obj.matrix_world
        for corner in bound_box:
            world_corners.append(matrix_world @ mathutils.Vector(corner))
    if not world_corners:
        return

    sorted_x = sorted(corner.x for corner in world_corners)
    sorted_y = sorted(corner.y for corner in world_corners)
    sorted_z = sorted(corner.z for corner in world_corners)
    corner_count = len(world_corners)
    low_index = max(0, int(corner_count * 0.02))
    high_index = min(corner_count - 1, int(corner_count * 0.98))

    min_corner = mathutils.Vector(
        (
            sorted_x[low_index],
            sorted_y[low_index],
            sorted_z[low_index],
        )
    )
    max_corner = mathutils.Vector(
        (
            sorted_x[high_index],
            sorted_y[high_index],
            sorted_z[high_index],
        )
    )
    focus_center = (min_corner + max_corner) * 0.5
    diagonal = (max_corner - min_corner).length
    if diagonal <= 1e-6:
        diagonal = 1.0

    # Baseline direction tuned from an approved manual viewport angle.
    view_direction = mathutils.Vector((-0.735, -0.487, -0.650)).normalized()
    target_rotation = view_direction.to_track_quat("-Z", "Y")

    prioritized_areas: list[bpy.types.Area] = []
    active_area = getattr(context, "area", None)
    if active_area is not None and active_area.type == "VIEW_3D":
        prioritized_areas.append(active_area)
    prioritized_areas.extend(
        area for area in screen.areas if area.type == "VIEW_3D" and area is not active_area
    )

    for area in prioritized_areas:
        region = next((region for region in area.regions if region.type == "WINDOW"), None)
        if region is None:
            continue
        with context.temp_override(window=window, area=area, region=region):
            bpy.ops.object.select_all(action="DESELECT")
            for obj in focus_objects:
                obj.select_set(True)
            _set_active_object(context, focus_objects[0])
            space = getattr(area, "spaces", None)
            active_space = space.active if space is not None else None
            region_3d = getattr(active_space, "region_3d", None)
            if region_3d is not None:
                region_3d.view_perspective = "PERSP"
                for _ in range(3):
                    if region_3d.view_perspective == "PERSP":
                        break
                    try:
                        bpy.ops.view3d.view_persportho()
                    except RuntimeError:
                        break
                    region_3d.view_perspective = "PERSP"
                region_3d.view_rotation = target_rotation
                region_3d.view_location = focus_center

                lens_mm = float(getattr(active_space, "lens", 50.0))
                lens_scale = max(lens_mm, 1.0) / 50.0
                # Deterministic fit: scales with lens so 100mm does not over-zoom.
                region_3d.view_distance = max(diagonal * 0.95 * lens_scale, diagonal * 0.78)


def _finalize_import_view(context: bpy.types.Context, package_root: bpy.types.Object) -> None:
    _collapse_template_cache_outliner(context)
    _frame_package_root_three_quarter(context, package_root)
    _force_view3d_perspective(context)
    bpy.ops.object.select_all(action="DESELECT")
    package_root.select_set(True)
    _set_active_object(context, package_root)


def _auto_apply_landing_gear_snap_last(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
) -> None:
    scene_path = package_root.get(PROP_SCENE_PATH)
    if not isinstance(scene_path, str) or not scene_path:
        return

    try:
        package = PackageBundle.load(scene_path)
    except Exception:
        return

    try:
        animation_items = available_package_animation_items(package)
    except Exception:
        return
    if not animation_items:
        return

    def _animation_score(name: str, display_name: str) -> int:
        text = f"{name} {display_name}".lower()
        has_landing_gear = "landing" in text and "gear" in text
        if not has_landing_gear:
            return -1

        score = 0
        if "retract" in text:
            score += 100
        if "stow" in text or "stowed" in text:
            score += 80
        if "close" in text or "closed" in text:
            score += 60
        if "up" in text:
            score += 40

        if "deploy" in text or "deployed" in text:
            score -= 60
        if "extend" in text or "extended" in text:
            score -= 60
        if "open" in text or "opened" in text:
            score -= 40
        if "down" in text:
            score -= 20
        return score

    best_name: str | None = None
    best_score = -1
    for animation_name, animation_display_name in animation_items:
        score = _animation_score(animation_name, animation_display_name)
        if score > best_score:
            best_score = score
            best_name = animation_name

    if best_name is None or best_score < 0:
        return

    try:
        apply_animation_mode_to_package_root(context, package_root, best_name, "snap_last")
    except Exception:
        return


def _update_pom_detail(_: bpy.types.ID, context: bpy.types.Context) -> None:
    scene = getattr(context, "scene", None)
    if scene is None:
        return
    try:
        apply_pom_detail_mode(getattr(scene, SCENE_POM_DETAIL_PROP, POM_DETAIL_DEFAULT))
    except Exception:
        return
    _tag_view3d_redraws(context)


def _draw_import_progress_overlay() -> None:
    context = bpy.context
    region = getattr(context, "region", None)
    if region is None:
        return
    window_manager = context.window_manager
    if not bool(getattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, False)):
        return

    fraction = _progress_fraction(getattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, 0.0))
    description = getattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, "Preparing import")

    panel_width = min(480.0, max(region.width - 80.0, 320.0))
    panel_height = 96.0
    panel_x = (region.width - panel_width) * 0.5
    panel_y = region.height * 0.12
    padding = 16.0
    bar_height = 24.0
    bar_width = panel_width - (padding * 2.0) - 72.0
    bar_x = panel_x + padding
    bar_y = panel_y + panel_height - padding - bar_height - 10.0

    shader = gpu.shader.from_builtin("UNIFORM_COLOR")

    def draw_rect(x: float, y: float, width: float, height: float, color: tuple[float, float, float, float]) -> None:
        vertices = ((x, y), (x + width, y), (x + width, y + height), (x, y + height))
        indices = ((0, 1, 2), (0, 2, 3))
        batch = batch_for_shader(shader, "TRIS", {"pos": vertices}, indices=indices)
        shader.bind()
        shader.uniform_float("color", color)
        batch.draw(shader)

    gpu.state.blend_set("ALPHA")
    draw_rect(panel_x, panel_y, panel_width, panel_height, (0.05, 0.07, 0.09, 0.88))
    draw_rect(panel_x + 1.0, panel_y + 1.0, panel_width - 2.0, panel_height - 2.0, (0.10, 0.12, 0.15, 0.92))
    draw_rect(bar_x, bar_y, bar_width, bar_height, (0.18, 0.21, 0.25, 1.0))
    if fraction > 0.0:
        draw_rect(bar_x, bar_y, bar_width * fraction, bar_height, (0.23, 0.62, 0.86, 1.0))
    gpu.state.blend_set("NONE")

    font_id = 0
    try:
        blf.size(font_id, 14.0)
    except TypeError:
        blf.size(font_id, 14, 72)
    blf.color(font_id, 0.96, 0.97, 0.98, 1.0)
    blf.position(font_id, bar_x + bar_width + 16.0, bar_y + 4.0, 0)
    blf.draw(font_id, f"{int(round(fraction * 100.0))}%")

    try:
        blf.size(font_id, 13.0)
    except TypeError:
        blf.size(font_id, 13, 72)
    blf.position(font_id, bar_x, panel_y + padding, 0)
    blf.draw(font_id, description)


def _ensure_import_progress_overlay() -> None:
    global _IMPORT_PROGRESS_DRAW_HANDLER
    if _IMPORT_PROGRESS_DRAW_HANDLER is not None:
        return
    _IMPORT_PROGRESS_DRAW_HANDLER = bpy.types.SpaceView3D.draw_handler_add(
        _draw_import_progress_overlay,
        (),
        "WINDOW",
        "POST_PIXEL",
    )


def _remove_import_progress_overlay() -> None:
    global _IMPORT_PROGRESS_DRAW_HANDLER
    if _IMPORT_PROGRESS_DRAW_HANDLER is None:
        return
    bpy.types.SpaceView3D.draw_handler_remove(_IMPORT_PROGRESS_DRAW_HANDLER, "WINDOW")
    _IMPORT_PROGRESS_DRAW_HANDLER = None


def _begin_import_progress(context: bpy.types.Context, description: str) -> None:
    global _IMPORT_PROGRESS_LAST_UPDATE
    window_manager = context.window_manager
    setattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, True)
    setattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, 0.0)
    setattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, description)
    _IMPORT_PROGRESS_LAST_UPDATE = 0.0
    _ensure_import_progress_overlay()
    _tag_view3d_redraws(context)
    try:
        window_manager.progress_begin(0, 100)
    except Exception:
        pass


def _update_import_progress(
    context: bpy.types.Context,
    fraction: float,
    description: str,
    *,
    force: bool = False,
) -> None:
    global _IMPORT_PROGRESS_LAST_UPDATE
    now = time.monotonic()
    if not force and now - _IMPORT_PROGRESS_LAST_UPDATE < 0.5:
        return
    window_manager = context.window_manager
    clamped = _progress_fraction(fraction)
    setattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, clamped)
    setattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, description)
    _IMPORT_PROGRESS_LAST_UPDATE = now
    try:
        window_manager.progress_update(int(round(clamped * 100.0)))
    except Exception:
        pass
    try:
        bpy.ops.wm.redraw_timer(type="DRAW_WIN_SWAP", iterations=1)
    except Exception:
        pass
    _tag_view3d_redraws(context)


def _end_import_progress(context: bpy.types.Context, description: str) -> None:
    window_manager = context.window_manager
    _update_import_progress(context, 1.0, description, force=True)
    setattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, False)
    _tag_view3d_redraws(context)
    try:
        window_manager.progress_end()
    except Exception:
        pass


def _package_root_from_context(context: bpy.types.Context) -> bpy.types.Object | None:
    package_root = find_package_root(context.active_object)
    if package_root is not None:
        return package_root
    for obj in context.selected_objects:
        package_root = find_package_root(obj)
        if package_root is not None:
            return package_root
    return None


def _selected_package(context: bpy.types.Context) -> PackageBundle | None:
    package_root = _package_root_from_context(context)
    if package_root is None:
        return None
    scene_path = package_root.get(PROP_SCENE_PATH)
    if not isinstance(scene_path, str) or not scene_path:
        return None
    try:
        return PackageBundle.load(scene_path)
    except Exception:
        return None


def _humanize_identifier(value: str) -> str:
    parts = [part for part in value.replace("-", "_").split("_") if part]
    words: list[str] = []
    for part in parts:
        lowered = part.lower()
        if lowered == "mk2":
            words.append("Mk2")
        elif lowered == "rsi":
            words.append("RSI")
        else:
            words.append(part.capitalize())
    return " ".join(words) if words else value


def _palette_display_name(palette_id: str, source_name: str | None, display_name: str | None) -> str:
    display_value = (display_name or "").strip()
    if display_value:
        return display_value
    source_key = (source_name or "").strip()
    if source_key:
        return _humanize_identifier(source_key)
    return _humanize_identifier(palette_id.split("/", 1)[-1])


def _paint_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    global _PAINT_ITEMS_CACHE
    package = _selected_package(context)
    if package is None:
        _PAINT_ITEMS_CACHE = [("", "No imported package", "Import a StarBreaker package first")]
        return _PAINT_ITEMS_CACHE
    available_ids = exterior_palette_ids(package)
    deduped_items: dict[str, tuple[str, str, str]] = {}
    for palette_id in available_ids:
        paint_variant = package.paints.get(palette_id)
        palette_entry = package.palettes.get(palette_id)
        if paint_variant is None and palette_entry is None:
            continue
        source_name = (
            (paint_variant.display_name if paint_variant else None)
            or (palette_entry.source_name if palette_entry else None)
            or palette_id
        )
        display_name_str = (
            (paint_variant.display_name if paint_variant else None)
            or (palette_entry.display_name if palette_entry else None)
        )
        description = (
            (paint_variant.subgeometry_tag if paint_variant else None)
            or source_name
        )
        item = (
            palette_id,
            _palette_display_name(palette_id, source_name, display_name_str),
            description,
        )
        canonical_id = paint_list_canonical_id(package, palette_id) or palette_id
        existing = deduped_items.get(canonical_id)
        if existing is not None and paint_variant is None:
            continue
        deduped_items[canonical_id] = item
    items = sorted(deduped_items.values(), key=lambda item: item[1])
    _PAINT_ITEMS_CACHE = items
    return _PAINT_ITEMS_CACHE


def _palette_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    global _PALETTE_ITEMS_CACHE
    package = _selected_package(context)
    if package is None:
        _PALETTE_ITEMS_CACHE = [("", "No imported package", "Import a StarBreaker package first")]
        return _PALETTE_ITEMS_CACHE
    _PALETTE_ITEMS_CACHE = [
        (
            palette_id,
            _palette_display_name(
                palette_id,
                package.palettes[palette_id].source_name,
                package.palettes[palette_id].display_name,
            ),
            package.palettes[palette_id].source_name or palette_id,
        )
        for palette_id in sorted(package.palettes.keys())
    ]
    return _PALETTE_ITEMS_CACHE


def _first_valid_item_id(items: list[tuple[str, str, str]]) -> str:
    for item_id, _, _ in items:
        if item_id:
            return item_id
    return ""


def _livery_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    global _LIVERY_ITEMS_CACHE
    package = _selected_package(context)
    if package is None:
        _LIVERY_ITEMS_CACHE = [("", "No imported package", "Import a StarBreaker package first")]
        return _LIVERY_ITEMS_CACHE
    _LIVERY_ITEMS_CACHE = [
        (livery_id, livery_id, package.liveries[livery_id].palette_source_name or livery_id)
        for livery_id in sorted(package.liveries.keys())
    ]
    return _LIVERY_ITEMS_CACHE


class STARBREAKER_OT_import_decomposed_package(Operator, ImportHelper):
    bl_idname = "starbreaker.import_decomposed_package"
    bl_label = "Import StarBreaker Package"
    bl_options = {"REGISTER", "UNDO"}

    _timer: bpy.types.Timer | None = None
    _started: bool = False

    filter_glob: StringProperty(default="scene.json;*.json", options={"HIDDEN"})
    prefer_cycles: BoolProperty(
        name="Prefer Cycles",
        description="Switch the active scene to Cycles before import",
        default=True,
    )
    palette_id_override: StringProperty(
        name="Initial Palette ID",
        description="Optional palette override applied during import to avoid rebuilding the package a second time",
        default="",
    )

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_name = Path(self.filepath).parent.name
        _begin_import_progress(context, f"Preparing {package_name}")
        self._started = False
        self._timer = context.window_manager.event_timer_add(0.01, window=context.window)
        context.window_manager.modal_handler_add(self)
        return {"RUNNING_MODAL"}

    def modal(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if event.type != "TIMER" or self._started:
            return {"PASS_THROUGH"}
        self._started = True
        package_name = Path(self.filepath).parent.name
        try:
            package_root = import_package(
                context,
                self.filepath,
                prefer_cycles=self.prefer_cycles,
                palette_id=self.palette_id_override.strip() or None,
                progress_callback=lambda fraction, description: _update_import_progress(
                    context,
                    fraction,
                    description,
                ),
            )
        except Exception as exc:
            _end_import_progress(context, f"Import failed: {exc}")
            self.cancel(context)
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        prefs = _get_prefs()
        if _should_apply_landing_gear(prefs):
            _auto_apply_landing_gear_snap_last(context, package_root)
        if _should_change_viewport(prefs):
            _finalize_import_view(context, package_root)
        _end_import_progress(context, f"Imported {package_root.get(PROP_PACKAGE_NAME, package_name)}")
        self.cancel(context)
        self.report({"INFO"}, f"Imported {package_root.get(PROP_PACKAGE_NAME, package_name)}")
        return {"FINISHED"}

    def cancel(self, context: bpy.types.Context) -> None:
        if self._timer is not None:
            context.window_manager.event_timer_remove(self._timer)
            self._timer = None


class STARBREAKER_OT_import_progress_popup(Operator):
    bl_idname = "starbreaker.import_progress_popup"
    bl_label = "StarBreaker Import Progress"
    bl_options = {"INTERNAL"}

    _timer: bpy.types.Timer | None = None
    _started: bool = False

    filepath: StringProperty(options={"HIDDEN"})
    package_name: StringProperty(options={"HIDDEN"})
    prefer_cycles: BoolProperty(options={"HIDDEN"}, default=True)
    palette_id_override: StringProperty(options={"HIDDEN"}, default="")

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        _begin_import_progress(context, f"Preparing {self.package_name or Path(self.filepath).parent.name}")
        self._started = False
        self._timer = context.window_manager.event_timer_add(0.01, window=context.window)
        context.window_manager.modal_handler_add(self)
        return context.window_manager.invoke_popup(self, width=420)

    def modal(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if event.type == "TIMER":
            if not self._started:
                self._started = True
                try:
                    package_root = import_package(
                        context,
                        self.filepath,
                        prefer_cycles=self.prefer_cycles,
                        palette_id=self.palette_id_override.strip() or None,
                        progress_callback=lambda fraction, description: _update_import_progress(
                            context,
                            fraction,
                            description,
                        ),
                    )
                except Exception as exc:
                    _end_import_progress(context, f"Import failed: {exc}")
                    self.cancel(context)
                    self.report({"ERROR"}, str(exc))
                    return {"CANCELLED"}
                prefs = _get_prefs()
                if _should_apply_landing_gear(prefs):
                    _auto_apply_landing_gear_snap_last(context, package_root)
                if _should_change_viewport(prefs):
                    _finalize_import_view(context, package_root)
                _end_import_progress(
                    context,
                    f"Imported {package_root.get(PROP_PACKAGE_NAME, self.package_name or Path(self.filepath).parent.name)}",
                )
                self.cancel(context)
                self.report(
                    {"INFO"},
                    f"Imported {package_root.get(PROP_PACKAGE_NAME, self.package_name or Path(self.filepath).parent.name)}",
                )
                return {"FINISHED"}
            if not getattr(context.window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, False):
                self.cancel(context)
                return {"FINISHED"}
            if context.window.screen is not None:
                for area in context.window.screen.areas:
                    area.tag_redraw()
        return {"PASS_THROUGH"}

    def cancel(self, context: bpy.types.Context) -> None:
        if self._timer is not None:
            context.window_manager.event_timer_remove(self._timer)
            self._timer = None

    def draw(self, context: bpy.types.Context) -> None:
        layout = self.layout
        window_manager = context.window_manager
        fraction = _progress_fraction(getattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, 0.0))
        description = getattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, "Preparing import")

        row = layout.row(align=True)
        bar = row.row()
        if hasattr(bar, "progress"):
            bar.progress(factor=fraction, type="BAR", text="")
        else:
            bar.prop(window_manager, _IMPORT_PROGRESS_VALUE_PROP, text="", slider=True)
        percent = row.row()
        percent.alignment = "RIGHT"
        percent.label(text=f"{int(round(fraction * 100.0))}%")
        layout.label(text=description)


class STARBREAKER_OT_apply_paint(Operator):
    bl_idname = "starbreaker.apply_paint"
    bl_label = "Apply Paint"
    bl_options = {"REGISTER", "UNDO"}

    paint_id: EnumProperty(name="Paint", items=_paint_items)

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        if not self.paint_id:
            self.report({"ERROR"}, "No paint selected")
            return {"CANCELLED"}
        apply_paint_to_selected_package(context, self.paint_id)
        self.report({"INFO"}, f"Applied paint {self.paint_id}")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if not self.paint_id:
            package_root = _package_root_from_context(context)
            current_palette_id = package_root.get(PROP_PALETTE_ID, "") if package_root is not None else ""
            item_ids = _paint_items(self, context)
            valid_ids = {item_id for item_id, _, _ in item_ids if item_id}
            if isinstance(current_palette_id, str) and current_palette_id in valid_ids:
                self.paint_id = current_palette_id
            else:
                self.paint_id = _first_valid_item_id(item_ids)
        return context.window_manager.invoke_props_dialog(self)


class STARBREAKER_OT_apply_palette(Operator):
    bl_idname = "starbreaker.apply_palette"
    bl_label = "Apply Palette"
    bl_options = {"REGISTER", "UNDO"}

    palette_id: EnumProperty(name="Palette", items=_palette_items)

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        if not self.palette_id:
            self.report({"ERROR"}, "No palette selected")
            return {"CANCELLED"}
        apply_palette_to_selected_package(context, self.palette_id)
        self.report({"INFO"}, f"Applied palette {self.palette_id}")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if not self.palette_id:
            package_root = _package_root_from_context(context)
            current_palette_id = package_root.get(PROP_PALETTE_ID, "") if package_root is not None else ""
            item_ids = _palette_items(self, context)
            valid_ids = {item_id for item_id, _, _ in item_ids if item_id}
            if isinstance(current_palette_id, str) and current_palette_id in valid_ids:
                self.palette_id = current_palette_id
            else:
                self.palette_id = _first_valid_item_id(item_ids)
        return context.window_manager.invoke_props_dialog(self)


class STARBREAKER_OT_apply_livery(Operator):
    bl_idname = "starbreaker.apply_livery"
    bl_label = "Apply Livery"
    bl_options = {"REGISTER", "UNDO"}

    livery_id: EnumProperty(name="Livery", items=_livery_items)

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        if not self.livery_id:
            self.report({"ERROR"}, "No livery selected")
            return {"CANCELLED"}
        applied = apply_livery_to_selected_package(context, self.livery_id)
        self.report({"INFO"}, f"Updated {applied} material slots")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if not self.livery_id:
            self.livery_id = _first_valid_item_id(_livery_items(self, context))
        return context.window_manager.invoke_props_dialog(self)


class STARBREAKER_OT_switch_light_state(Operator):
    bl_idname = "starbreaker.switch_light_state"
    bl_label = "Switch Light State"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = (
        "Switch every imported StarBreaker light to the named CryEngine "
        "authored state (defaultState, auxiliaryState, emergencyState, "
        "cinematicState). Lights that lack the requested state keep their "
        "current values."
    )

    state_name: StringProperty(name="State")  # type: ignore[assignment]

    def execute(self, context: bpy.types.Context) -> set[str]:
        name = (self.state_name or "").strip()
        if not name:
            self.report({"ERROR"}, "No state name provided")
            return {"CANCELLED"}
        count = apply_light_state(name)
        self.report({"INFO"}, f"Applied '{name}' to {count} light(s)")
        return {"FINISHED"}


class STARBREAKER_OT_dump_metadata(Operator):
    bl_idname = "starbreaker.dump_metadata"
    bl_label = "Dump Metadata"
    bl_options = {"REGISTER"}

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return context.active_object is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        try:
            text_names = dump_selected_metadata(context)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        if not text_names:
            self.report({"WARNING"}, "No StarBreaker metadata found on the current selection")
            return {"CANCELLED"}
        self.report({"INFO"}, f"Created {len(text_names)} text datablocks")
        return {"FINISHED"}


class STARBREAKER_OT_apply_animation_mode(Operator):
    bl_idname = "starbreaker.apply_animation_mode"
    bl_label = "Apply Animation Mode"
    bl_options = {"REGISTER", "UNDO"}

    animation_name: StringProperty(name="Animation")  # type: ignore[assignment]
    mode: EnumProperty(name="Mode", items=_ANIMATION_MODE_ITEMS)  # type: ignore[assignment]
    fps_policy: EnumProperty(  # type: ignore[assignment]
        name="FPS Policy",
        items=_ANIMATION_FPS_POLICY_ITEMS,
        default="adapt_scene",
    )

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}
        name = (self.animation_name or "").strip()
        if not name:
            self.report({"ERROR"}, "No animation selected")
            return {"CANCELLED"}
        try:
            updated = apply_animation_mode_to_package_root(
                context,
                package_root,
                name,
                self.mode,
                fps_policy=self.fps_policy,
            )
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        self.report({"INFO"}, f"{name}: {self.mode} ({updated} object(s) updated)")
        return {"FINISHED"}


class STARBREAKER_OT_dump_animation_diagnostics(Operator):
    bl_idname = "starbreaker.dump_animation_diagnostics"
    bl_label = "Animation Diagnostics"
    bl_options = {"REGISTER"}
    bl_description = "Dump hash/object matching diagnostics for one animation"

    animation_name: StringProperty(name="Animation")  # type: ignore[assignment]

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return _package_root_from_context(context) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}

        package = _selected_package(context)
        if package is None:
            self.report({"ERROR"}, "Unable to load package from selected object")
            return {"CANCELLED"}

        name = (self.animation_name or "").strip()
        if not name:
            self.report({"ERROR"}, "No animation selected")
            return {"CANCELLED"}

        try:
            diagnostics = package_animation_diagnostics(package, package_root, name)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}

        text_name = f"starbreaker_anim_diag_{Path(name).stem}.json"
        text = bpy.data.texts.get(text_name)
        if text is None:
            text = bpy.data.texts.new(text_name)
        else:
            text.clear()
        text.from_string(json.dumps(diagnostics, indent=2, sort_keys=True))

        self.report(
            {"INFO"},
            (
                f"{name}: {diagnostics['matched_object_count']} objects, "
                f"{diagnostics['unmatched_hash_count']} unmatched hashes "
                f"(saved to {text.name})"
            ),
        )
        return {"FINISHED"}


class STARBREAKER_PT_tools(Panel):
    bl_label = "StarBreaker"
    bl_idname = "STARBREAKER_PT_tools"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "StarBreaker"

    def draw(self, context: bpy.types.Context) -> None:
        layout = self.layout
        layout.operator(STARBREAKER_OT_import_decomposed_package.bl_idname, icon="IMPORT")

        obj = context.active_object
        package_root = _package_root_from_context(context)
        if package_root is None:
            return

        package = _selected_package(context)
        info = layout.box()
        info.label(text=f"Package: {package_root.get(PROP_PACKAGE_NAME, '')}")
        info.label(text=f"Entity: {obj.get(PROP_ENTITY_NAME, obj.name) if obj else ''}")
        info.label(text=f"Palette: {package_root.get(PROP_PALETTE_ID, '')}")
        if obj is not None:
            material_sidecar = obj.get(PROP_MATERIAL_SIDECAR)
            if isinstance(material_sidecar, str) and material_sidecar:
                info.label(text=f"Sidecar: {Path(material_sidecar).name}")

        actions = layout.row(align=True)
        actions.operator_menu_enum(STARBREAKER_OT_apply_paint.bl_idname, "paint_id", text="Apply Paint", icon="BRUSH_DATA")
        layout.operator(STARBREAKER_OT_dump_metadata.bl_idname, icon="TEXT")

        tuning = layout.box()
        tuning.prop(context.scene, SCENE_POM_DETAIL_PROP, text="POM Detail")
        tuning.label(text="Layered Wear")
        tuning.prop(context.scene, SCENE_WEAR_STRENGTH_PROP, slider=True)

        if package is not None:
            available = layout.box()
            available.label(text=f"Palettes: {', '.join(sorted(package.palettes.keys()))}")
            available.label(text=f"Liveries: {', '.join(sorted(package.liveries.keys()))}")

        if obj is not None and obj.active_material is not None:
            material = obj.active_material
            material_box = layout.box()
            material_box.label(text=f"Shader: {material.get(PROP_SHADER_FAMILY, '')}")
            material_box.label(text=f"Template: {material.get(PROP_TEMPLATE_KEY, '')}")
            material_box.label(text=f"Surface: {material.get(PROP_SURFACE_SHADER_MODE, '')}")

        # Phase 28: light state switcher. Show a row of buttons when the
        # current .blend has any imported lights with authored states.
        state_names = available_light_state_names()
        if state_names:
            light_box = layout.box()
            light_box.label(text="Light States")
            row = light_box.row(align=True)
            _SHORT = {
                "defaultState": "Default",
                "auxiliaryState": "Auxiliary",
                "emergencyState": "Emergency",
                "cinematicState": "Cinematic",
                "offState": "Off",
            }
            for name in state_names:
                op = row.operator(
                    STARBREAKER_OT_switch_light_state.bl_idname,
                    text=_SHORT.get(name, name),
                )
                op.state_name = name

        if package is not None:
            animation_items = available_package_animation_items(package)
            animation_box = layout.box()
            animation_box.label(text="Animations")
            if not animation_items:
                animation_box.label(text="No animations exported in this scene.json")
            else:
                animation_box.prop(context.scene, _SCENE_ANIMATION_FPS_POLICY_PROP, text="FPS")
                mode_map = package_animation_mode_map(package_root)
                fps_policy = getattr(context.scene, _SCENE_ANIMATION_FPS_POLICY_PROP, "adapt_scene")
                for animation_name, animation_display_name in animation_items:
                    animation_box.label(text=animation_display_name)
                    current_mode = mode_map.get(animation_name, "none")
                    buttons_row = animation_box.row(align=True)
                    for mode_id, mode_label, _ in _ANIMATION_MODE_ITEMS:
                        op = buttons_row.operator(
                            STARBREAKER_OT_apply_animation_mode.bl_idname,
                            text=mode_label,
                            depress=(current_mode == mode_id),
                        )
                        op.animation_name = animation_name
                        op.mode = mode_id
                        op.fps_policy = fps_policy


# ── Addon Preferences ────────────────────────────────────────────────────────

class STARBREAKER_AP_preferences(bpy.types.AddonPreferences):
    """User-configurable post-import behaviour for the StarBreaker addon."""

    bl_idname = "starbreaker_addon"

    viewport_change_after_import: BoolProperty(  # type: ignore[assignment]
        name="Adjust viewport after import",
        description=(
            "Frame the imported package in the 3D viewport and switch to "
            "perspective view after a successful import."
        ),
        default=True,
    )

    landing_gear_retract_after_import: BoolProperty(  # type: ignore[assignment]
        name="Retract landing gear after import",
        description=(
            "Automatically apply the best landing-gear retracted pose when a "
            "package is imported, so ships appear ready-for-flight by default."
        ),
        default=True,
    )

    def draw(self, context: bpy.types.Context) -> None:
        layout = self.layout
        layout.label(text="Post-import behaviour:")
        layout.prop(self, "landing_gear_retract_after_import")
        layout.prop(self, "viewport_change_after_import")


def _get_prefs() -> STARBREAKER_AP_preferences | None:
    """Return the addon preferences object, or None when not available."""
    addon_entry = bpy.context.preferences.addons.get("starbreaker_addon")
    if addon_entry is None:
        return None
    return getattr(addon_entry, "preferences", None)


def _should_apply_landing_gear(prefs: object | None) -> bool:
    """Pure gate — return True when landing-gear retract should run after import."""
    if prefs is None:
        return True
    return bool(getattr(prefs, "landing_gear_retract_after_import", True))


def _should_change_viewport(prefs: object | None) -> bool:
    """Pure gate — return True when viewport framing should run after import."""
    if prefs is None:
        return True
    return bool(getattr(prefs, "viewport_change_after_import", True))


CLASSES = [
    STARBREAKER_AP_preferences,
    STARBREAKER_OT_import_decomposed_package,
    STARBREAKER_OT_import_progress_popup,
    STARBREAKER_OT_apply_paint,
    STARBREAKER_OT_apply_palette,
    STARBREAKER_OT_apply_livery,
    STARBREAKER_OT_switch_light_state,
    STARBREAKER_OT_dump_metadata,
    STARBREAKER_OT_apply_animation_mode,
    STARBREAKER_OT_dump_animation_diagnostics,
    STARBREAKER_PT_tools,
]


def register() -> None:
    setattr(bpy.types.WindowManager, _IMPORT_PROGRESS_ACTIVE_PROP, BoolProperty(default=False))
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_VALUE_PROP,
        FloatProperty(default=0.0, min=0.0, max=1.0),
    )
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_DESCRIPTION_PROP,
        StringProperty(default="Preparing import"),
    )
    setattr(
        bpy.types.Scene,
        SCENE_POM_DETAIL_PROP,
        EnumProperty(
            name="POM Detail",
            description=(
                "Global quality preset for StarBreaker parallax-occlusion materials. "
                "Updates the shared runtime POM detail group so imported POM materials "
                "change quality together without rewriting each material node tree."
            ),
            items=POM_DETAIL_ITEMS,
            default=POM_DETAIL_DEFAULT,
            update=_update_pom_detail,
        ),
    )
    setattr(
        bpy.types.Scene,
        SCENE_WEAR_STRENGTH_PROP,
        FloatProperty(
            name="Wear Strength",
            description=(
                "Scale layered wear contribution for imported StarBreaker "
                "layered materials. Default is 0 because vertex-colour-driven "
                "wear on ship hulls would otherwise blend the primary paint "
                "toward a worn-grey layer on every import, which does not "
                "match the default in-game appearance of a freshly spawned "
                "ship. Raise this slider to expose the authored wear layer."
            ),
            default=0.0,
            min=0.0,
            max=2.0,
            soft_min=0.0,
            soft_max=2.0,
        ),
    )
    setattr(
        bpy.types.Scene,
        _SCENE_ANIMATION_FPS_POLICY_PROP,
        EnumProperty(
            name="Animation FPS",
            description=(
                "How to reconcile clip FPS with scene FPS when inserting animation actions. "
                "Snap modes are unaffected."
            ),
            items=_ANIMATION_FPS_POLICY_ITEMS,
            default="adapt_scene",
        ),
    )
    for cls in CLASSES:
        bpy.utils.register_class(cls)


def unregister() -> None:
    _remove_import_progress_overlay()
    for cls in reversed(CLASSES):
        try:
            bpy.utils.unregister_class(cls)
        except RuntimeError:
            pass
    for prop_name in (
        _IMPORT_PROGRESS_ACTIVE_PROP,
        _IMPORT_PROGRESS_VALUE_PROP,
        _IMPORT_PROGRESS_DESCRIPTION_PROP,
    ):
        if hasattr(bpy.types.WindowManager, prop_name):
            delattr(bpy.types.WindowManager, prop_name)
    if hasattr(bpy.types.Scene, SCENE_POM_DETAIL_PROP):
        delattr(bpy.types.Scene, SCENE_POM_DETAIL_PROP)
    if hasattr(bpy.types.Scene, SCENE_WEAR_STRENGTH_PROP):
        delattr(bpy.types.Scene, SCENE_WEAR_STRENGTH_PROP)
    if hasattr(bpy.types.Scene, _SCENE_ANIMATION_FPS_POLICY_PROP):
        delattr(bpy.types.Scene, _SCENE_ANIMATION_FPS_POLICY_PROP)
