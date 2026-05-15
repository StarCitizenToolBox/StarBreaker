"""Tests for the load-post automatic material refresh timer behavior."""

from __future__ import annotations

import ast
import types
import unittest
from pathlib import Path


ADDON_ROOT = Path(__file__).resolve().parents[1]


def _load_ui_function(name: str):
    ui_path = ADDON_ROOT / "starbreaker_addon" / "ui.py"
    source = ui_path.read_text(encoding="utf-8")
    tree = ast.parse(source)

    namespace: dict = {
        "_AUTO_MATERIAL_REFRESH_SESSION": None,
        "_AUTO_MATERIAL_REFRESH_TOKEN": 1,
        "PROP_PACKAGE_NAME": "starbreaker_package_name",
        "PROP_PACKAGE_ROOT": "starbreaker_package_root",
        "PROP_PALETTE_ID": "starbreaker_palette_id",
        "bpy": types.SimpleNamespace(
            context=object(),
            data=types.SimpleNamespace(objects=[]),
            app=types.SimpleNamespace(timers=types.SimpleNamespace(register=lambda *args, **kwargs: None)),
        ),
    }
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name == name:
            func_source = ast.get_source_segment(source, node)
            if func_source:
                exec(compile(ast.parse(func_source), str(ui_path), "exec"), namespace)  # noqa: S102
                break
    return namespace[name], namespace


def _load_material_refresh_prompt_timer():
    return _load_ui_function("_material_refresh_prompt_timer")


def _load_loaded_package_roots():
    roots_func, namespace = _load_ui_function("_loaded_package_roots")
    depth_func, _ = _load_ui_function("_object_parent_depth")
    namespace["_object_parent_depth"] = depth_func
    return roots_func, namespace


def _load_open_view3d_sidebar():
    return _load_ui_function("_open_view3d_sidebar")


class _MockObject(dict):
    def __init__(self, name: str, **props):
        super().__init__(props)
        self.name = name
        self.parent = None
        self.selected = False

    def select_set(self, value: bool) -> None:
        self.selected = value


class TestAutoRefreshPromptTimer(unittest.TestCase):
    def test_refreshes_first_eligible_root_synchronously(self) -> None:
        prompt_timer, namespace = _load_material_refresh_prompt_timer()
        root = _MockObject(
            "StarBreaker Test",
            starbreaker_package_root=True,
            starbreaker_package_name="Test Package",
            starbreaker_palette_id="palette/test",
        )
        refresh_calls: list[tuple[object, object, str | None, bool, object, object, float]] = []
        begin_calls: list[tuple[str, dict]] = []
        end_calls: list[str] = []
        session_created: list[tuple[object, object, str | None]] = []
        focus_calls: list[object] = []
        deferred_focus_calls: list[tuple[object, float, bool]] = []

        namespace["bpy"].data.objects = [root]
        namespace["_get_prefs"] = lambda: object()
        namespace["_should_auto_refresh_unloaded_materials_on_load"] = lambda _prefs: True
        namespace["_loaded_package_roots"] = lambda _objects, max_depth=2: [root]
        namespace["_focus_loaded_package_root"] = lambda _context, package_root: focus_calls.append(package_root)
        namespace["_deferred_focus_loaded_package_root"] = lambda _root_name: None
        namespace["dirty_package_material_objects"] = lambda obj: [obj]
        namespace["_begin_import_progress"] = (
            lambda _context, description, **kwargs: begin_calls.append((description, kwargs))
        )
        namespace["_end_import_progress"] = lambda _context, description: end_calls.append(description)
        namespace["refresh_materials_for_package_root"] = (
            lambda context, package_root, palette_id, *, only_unloaded=False, target_objects=None, progress_callback=None, progress_interval_seconds=0.0: refresh_calls.append(
                (context, package_root, palette_id, only_unloaded, target_objects, progress_callback, progress_interval_seconds)
            )
            or 7
        )
        namespace["bpy"].ops = types.SimpleNamespace(
            wm=types.SimpleNamespace(redraw_timer=lambda **_kwargs: None)
        )
        namespace["bpy"].app.timers.register = (
            lambda callback, *, first_interval=0.0, persistent=False: deferred_focus_calls.append(
                (callback, first_interval, persistent)
            )
        )

        class _UnexpectedSession:
            def __init__(self, context, package_root, palette_id=None):
                session_created.append((context, package_root, palette_id))

        namespace["MaterialRefreshSession"] = _UnexpectedSession

        result = prompt_timer(token=1)

        self.assertIsNone(result)
        self.assertEqual(len(refresh_calls), 1)
        self.assertEqual(refresh_calls[0][:5], (namespace["bpy"].context, root, "palette/test", True, [root]))
        self.assertTrue(callable(refresh_calls[0][5]))
        self.assertEqual(refresh_calls[0][6], 5.0)
        self.assertEqual(focus_calls, [root])
        self.assertEqual(len(deferred_focus_calls), 1)
        self.assertEqual(deferred_focus_calls[0][1:], (0.25, False))
        self.assertEqual(
            begin_calls,
            [("Loading materials, please wait", {"indeterminate": True, "text_only": True})],
        )
        self.assertEqual(end_calls, ["Refreshed 7 material slots for Test Package"])
        self.assertEqual(session_created, [])
        self.assertIsNone(namespace["_AUTO_MATERIAL_REFRESH_SESSION"])

    def test_skips_refresh_when_pref_disabled(self) -> None:
        prompt_timer, namespace = _load_material_refresh_prompt_timer()
        root = _MockObject(
            "StarBreaker Test",
            starbreaker_package_root=True,
            starbreaker_package_name="Test Package",
        )
        refresh_calls: list[tuple[object, object, str | None, bool, object, object, float]] = []
        namespace["bpy"].data.objects = [root]
        namespace["_get_prefs"] = lambda: object()
        namespace["_should_auto_refresh_unloaded_materials_on_load"] = lambda _prefs: False
        namespace["_loaded_package_roots"] = lambda _objects, max_depth=2: [root]
        namespace["_focus_loaded_package_root"] = lambda *_args: None
        namespace["_deferred_focus_loaded_package_root"] = lambda _root_name: None
        namespace["dirty_package_material_objects"] = lambda obj: [obj]
        namespace["_begin_import_progress"] = lambda *_args, **_kwargs: None
        namespace["_end_import_progress"] = lambda *_args: None
        namespace["refresh_materials_for_package_root"] = (
            lambda context, package_root, palette_id, *, only_unloaded=False, target_objects=None, progress_callback=None, progress_interval_seconds=0.0: refresh_calls.append(
                (context, package_root, palette_id, only_unloaded, target_objects, progress_callback, progress_interval_seconds)
            )
            or 1
        )

        result = prompt_timer(token=1)

        self.assertIsNone(result)
        self.assertEqual(refresh_calls, [])

    def test_loaded_package_roots_filters_by_parent_depth(self) -> None:
        loaded_package_roots, _namespace = _load_loaded_package_roots()

        shallow = _MockObject("Shallow", starbreaker_package_root=True)
        child = _MockObject("Child")
        child.parent = shallow
        deep_parent = _MockObject("DeepParent")
        deep_root_parent = _MockObject("DeepRootParent")
        deepest_parent = _MockObject("DeepestParent")
        too_deep = _MockObject("TooDeep", starbreaker_package_root=True)
        too_deep.parent = deep_parent
        deep_parent.parent = deep_root_parent
        deep_root_parent.parent = deepest_parent
        ignored = _MockObject("Ignored", starbreaker_package_root=False)

        roots = loaded_package_roots([too_deep, ignored, shallow], max_depth=2)

        self.assertEqual([obj.name for obj in roots], ["Shallow"])

    def test_open_view3d_sidebar_selects_starbreaker_category(self) -> None:
        open_view3d_sidebar, _namespace = _load_open_view3d_sidebar()

        class _MockRegion:
            def __init__(self, type: str):
                self.type = type
                self.active_panel_category = "View"

        class _MockSpace:
            def __init__(self):
                self.show_region_ui = False

        class _MockArea:
            def __init__(self, type: str):
                self.type = type
                self.regions = [_MockRegion("UI")]
                self.spaces = types.SimpleNamespace(active=_MockSpace())
                self.redraws = 0

            def tag_redraw(self) -> None:
                self.redraws += 1

        view3d = _MockArea("VIEW_3D")
        other = _MockArea("OUTLINER")
        context = types.SimpleNamespace(
            window=types.SimpleNamespace(screen=types.SimpleNamespace(areas=[view3d, other]))
        )

        open_view3d_sidebar(context)

        self.assertTrue(view3d.spaces.active.show_region_ui)
        self.assertEqual(view3d.regions[0].active_panel_category, "StarBreaker")
        self.assertEqual(view3d.redraws, 1)


if __name__ == "__main__":
    unittest.main()
