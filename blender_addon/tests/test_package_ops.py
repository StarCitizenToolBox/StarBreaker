from __future__ import annotations

from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


class FakeObject(dict):
    def __init__(self, name: str, **props):
        super().__init__(props)
        self.name = name
        self.parent = None
        self.children: list[FakeObject] = []
        self.type = "EMPTY"
        self.material_slots = []

    @property
    def children_recursive(self) -> list[FakeObject]:
        result: list[FakeObject] = []
        stack = list(self.children)
        while stack:
            child = stack.pop()
            result.append(child)
            stack.extend(child.children)
        return result


class FakeObjects(list):
    def __init__(self, items: list[FakeObject] | None = None):
        super().__init__(items or [])
        self.removed: list[tuple[str, bool]] = []

    def remove(self, obj, do_unlink: bool = False):  # noqa: A003 - matches bpy API name
        self.removed.append((obj.name, do_unlink))
        if obj in self:
            super().remove(obj)


class FakeMaterial(dict):
    def __init__(self, name: str, *, library=None, **props):
        super().__init__(props)
        self.name = name
        self.library = library


class FakeSlot:
    def __init__(self, material=None):
        self.material = material


class FakeUVLayer:
    def __init__(self, name: str):
        self.name = name


class FakeUVLayers(list):
    def __init__(self, names: list[str], active_index: int = -1):
        super().__init__(FakeUVLayer(name) for name in names)
        self.active_index = active_index

    @property
    def active(self):
        if 0 <= self.active_index < len(self):
            return self[self.active_index]
        return None

    def find(self, name: str) -> int:
        for index, layer in enumerate(self):
            if layer.name == name:
                return index
        return -1


class FakeMeshData:
    def __init__(self, uv_layers: FakeUVLayers | None = None):
        self.uv_layers = uv_layers or FakeUVLayers([])
        self.update_tags: list[object] = []

    def update_tag(self, refresh=None):
        self.update_tags.append(refresh)


class FakeViewLayer:
    def __init__(self):
        self.update_count = 0

    def update(self):
        self.update_count += 1


def _load_package_ops() -> tuple[types.ModuleType, types.ModuleType]:
    bpy = sys.modules.get("bpy")
    if bpy is None:
        bpy = types.ModuleType("bpy")
        sys.modules["bpy"] = bpy
    bpy.types = types.SimpleNamespace(Context=object, Object=object, ID=object, Light=object)
    bpy.data = types.SimpleNamespace(objects=FakeObjects(), lights=[], filepath="")

    mathutils = sys.modules.get("mathutils")
    if mathutils is None:
        mathutils = types.ModuleType("mathutils")
        sys.modules["mathutils"] = mathutils

    class Matrix(tuple):
        def __new__(cls, rows):
            return tuple.__new__(cls, rows)

        def inverted(self):
            return self

    class Quaternion(tuple):
        def __new__(cls, values):
            return tuple.__new__(cls, values)

    mathutils.Matrix = Matrix
    mathutils.Quaternion = Quaternion

    def _load(name: str, path: Path) -> types.ModuleType:
        spec = __import__("importlib.util").util.spec_from_file_location(name, str(path))
        assert spec is not None and spec.loader is not None
        module = __import__("importlib.util").util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
        return module

    constants = _load("sb_pkg_test_runtime.constants", ADDON_ROOT / "runtime" / "constants.py")
    runtime_pkg = types.ModuleType("sb_pkg_test_runtime")
    runtime_pkg.__path__ = [str(ADDON_ROOT / "runtime")]
    sys.modules["sb_pkg_test_runtime"] = runtime_pkg

    addon_pkg = types.ModuleType("sb_pkg_test_addon")
    addon_pkg.__path__ = [str(ADDON_ROOT)]
    sys.modules["sb_pkg_test_addon"] = addon_pkg

    manifest_stub = types.ModuleType("sb_pkg_test_addon.manifest")

    class PackageBundle:
        @staticmethod
        def load(scene_path):
            return types.SimpleNamespace(
                scene_path=Path(scene_path),
                package_name="Test Package",
                load_material_sidecar=lambda _path: None,
            )

    manifest_stub.PackageBundle = PackageBundle
    manifest_stub.SceneInstanceRecord = type("SceneInstanceRecord", (), {})
    sys.modules["sb_pkg_test_addon.manifest"] = manifest_stub

    palette_stub = types.ModuleType("sb_pkg_test_addon.palette")
    palette_stub.palette_id_for_livery_instance = lambda *args, **kwargs: None
    palette_stub.resolved_palette_id = lambda package, requested, inherited: requested or inherited
    sys.modules["sb_pkg_test_addon.palette"] = palette_stub

    validators_stub = types.ModuleType("sb_pkg_test_runtime.validators")
    validators_stub._purge_orphaned_file_backed_images = lambda: 0
    validators_stub._purge_orphaned_runtime_groups = lambda: 0
    sys.modules["sb_pkg_test_runtime.validators"] = validators_stub

    importer_stub = types.ModuleType("sb_pkg_test_runtime.importer")
    importer_stub.events = []

    class PackageImporter:
        def __init__(self, context, package, progress_callback=None, package_root=None):
            self.context = context
            self.package = package
            self.progress_callback = progress_callback
            self.package_root = package_root

        def import_scene(self, prefer_cycles=True, palette_id=None):
            importer_stub.events.append(("import", str(self.package.scene_path), prefer_cycles, palette_id))
            return "imported-root"

        def rebuild_object_materials(self, obj, palette_id):
            importer_stub.events.append(("rebuild", obj.name, palette_id))
            return 1

    importer_stub.PackageImporter = PackageImporter
    sys.modules["sb_pkg_test_runtime.importer"] = importer_stub

    source = (ADDON_ROOT / "runtime" / "package_ops.py").read_text()
    source = source.replace("from ..manifest import", "from sb_pkg_test_addon.manifest import")
    source = source.replace("from ..palette import", "from sb_pkg_test_addon.palette import")
    source = source.replace("from .constants import", "from sb_pkg_test_runtime.constants import")
    source = source.replace("from .validators import", "from sb_pkg_test_runtime.validators import")
    source = source.replace("from .importer import PackageImporter", "from sb_pkg_test_runtime.importer import PackageImporter")
    module = types.ModuleType("sb_pkg_test_runtime.package_ops")
    module.__file__ = str(ADDON_ROOT / "runtime" / "package_ops.py")
    module.__package__ = "sb_pkg_test_runtime"
    sys.modules[module.__name__] = module
    exec(compile(source, module.__file__, "exec"), module.__dict__)
    return module, bpy


class PackageOpsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.package_ops, cls.bpy = _load_package_ops()
        cls._original_remove_existing_package_instances = cls.package_ops._remove_existing_package_instances
        cls._original_suspend_heavy_viewports = cls.package_ops._suspend_heavy_viewports

    def setUp(self) -> None:
        self.bpy.data.objects = FakeObjects()
        self.package_ops._remove_existing_package_instances = type(self)._original_remove_existing_package_instances
        self.package_ops._suspend_heavy_viewports = type(self)._original_suspend_heavy_viewports

    def test_remove_existing_package_instances_replaces_matching_scene_path(self) -> None:
        scene_path = Path("/tmp/vulture/scene.json")
        other_scene_path = Path("/tmp/aurora/scene.json")
        package_root = FakeObject(
            "StarBreaker DRAK Vulture",
            starbreaker_package_root=True,
            starbreaker_scene_path=str(scene_path),
        )
        child = FakeObject("Vulture Child")
        child.parent = package_root
        package_root.children.append(child)
        other_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path=str(other_scene_path),
        )
        self.bpy.data.objects.extend([package_root, child, other_root])

        removed = self.package_ops._remove_existing_package_instances(scene_path)

        self.assertEqual(removed, 2)
        self.assertEqual(self.bpy.data.objects.removed, [("Vulture Child", True), ("StarBreaker DRAK Vulture", True)])
        self.assertEqual(list(self.bpy.data.objects), [other_root])

    def test_import_package_removes_existing_package_before_import(self) -> None:
        events: list[tuple[str, str]] = []

        def _cleanup(scene_path):
            events.append(("cleanup", str(scene_path)))
            return 1

        @contextmanager
        def _no_suspend(_context):
            yield

        self.package_ops._remove_existing_package_instances = _cleanup
        self.package_ops._suspend_heavy_viewports = _no_suspend
        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []

        context = types.SimpleNamespace(scene=types.SimpleNamespace(render=types.SimpleNamespace(engine="BLENDER_EEVEE")))
        root = self.package_ops.import_package(context, "/tmp/vulture/scene.json", prefer_cycles=False, palette_id="palette/test")

        self.assertEqual(root, "imported-root")
        self.assertEqual(events, [("cleanup", "/tmp/vulture/scene.json")])
        self.assertEqual(
            importer_stub.events,
            [("import", "/tmp/vulture/scene.json", False, "palette/test")],
        )

    def test_refresh_materials_rebuilds_only_meshes_with_sidecars(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh = FakeObject("hull", starbreaker_material_sidecar="hull.materials.json")
        mesh.type = "MESH"
        empty = FakeObject("helper", starbreaker_material_sidecar="helper.materials.json")
        empty.type = "EMPTY"
        mesh.parent = package_root
        empty.parent = package_root
        package_root.children.extend([mesh, empty])

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(
                types.SimpleNamespace(),
                package_root,
                "palette/test",
            )
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(importer_stub.events, [("rebuild", "hull", "palette/test")])
        self.assertEqual(package_root["starbreaker_palette_id"], "palette/test")
        self.assertEqual(mesh["starbreaker_palette_id"], "palette/test")

    def test_refresh_materials_uses_object_palette_without_explicit_override(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
            starbreaker_palette_id="palette/root",
        )
        mesh = FakeObject(
            "interior_panel",
            starbreaker_material_sidecar="interior.materials.json",
            starbreaker_palette_id="palette/interior",
        )
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(types.SimpleNamespace(), package_root)
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(importer_stub.events, [("rebuild", "interior_panel", "palette/interior")])
        self.assertEqual(package_root["starbreaker_palette_id"], "palette/root")
        self.assertEqual(mesh["starbreaker_palette_id"], "palette/interior")

    def test_refresh_materials_uses_sidecar_default_palette_when_object_palette_missing(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh = FakeObject(
            "interior_panel",
            starbreaker_material_sidecar="interior.materials.json",
        )
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)
        sidecar = types.SimpleNamespace(
            raw={
                "authored_material_set": {
                    "attributes": [
                        {
                            "name": "DefaultPalette",
                            "value": "Libs/Foundry/Records/TintPalettes/Brand/RSI/rsi_interior/rsi_interior_default",
                        }
                    ]
                }
            }
        )
        fake_package = types.SimpleNamespace(
            scene_path=Path("/tmp/aurora/scene.json"),
            package_name="Test Package",
            load_material_sidecar=lambda _path: sidecar,
        )

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_loader = self.package_ops._load_package_from_root
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._load_package_from_root = lambda _root: fake_package
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(types.SimpleNamespace(), package_root)
        finally:
            self.package_ops._load_package_from_root = original_loader
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(importer_stub.events, [("rebuild", "interior_panel", "palette/rsi_interior_default")])

    def test_refresh_materials_sets_active_uv_without_localizing_mesh(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh = FakeObject("wing", starbreaker_material_sidecar="wing.materials.json")
        mesh.type = "MESH"
        mesh.data = FakeMeshData(FakeUVLayers(["UVMap.001", "UVMap"]))
        mesh.update_tags: list[object] = []
        mesh.update_tag = lambda refresh=None: mesh.update_tags.append(refresh)
        mesh.parent = package_root
        package_root.children.append(mesh)
        view_layer = FakeViewLayer()

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(
                types.SimpleNamespace(view_layer=view_layer),
                package_root,
            )
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(mesh.data.uv_layers.active.name, "UVMap")
        self.assertEqual(mesh.data.update_tags, [{"DATA"}])
        self.assertEqual(mesh.update_tags, [{"DATA"}])
        self.assertEqual(view_layer.update_count, 1)

    def test_package_root_needs_material_refresh_for_linked_or_unmanaged_slots(self) -> None:
        package_root = FakeObject("Root", starbreaker_package_root=True)
        mesh = FakeObject("mesh", starbreaker_material_sidecar="mesh.materials.json")
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)

        self.assertTrue(self.package_ops.package_root_needs_material_refresh(package_root))

        mesh.material_slots = [FakeSlot(FakeMaterial("linked", library=object(), starbreaker_material_identity="id"))]
        self.assertTrue(self.package_ops.package_root_needs_material_refresh(package_root))

        mesh.material_slots = [FakeSlot(FakeMaterial("local"))]
        self.assertTrue(self.package_ops.package_root_needs_material_refresh(package_root))

        mesh.material_slots = [FakeSlot(FakeMaterial("local", starbreaker_material_identity="id"))]
        self.assertFalse(self.package_ops.package_root_needs_material_refresh(package_root))

    def test_resolve_package_relative_scene_path_from_opened_blend_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            scene_path = root / "Packages" / "RSI Aurora Mk2_LOD0_TEX0" / "scene.json"
            scene_path.parent.mkdir(parents=True)
            scene_path.write_text("{}", encoding="utf-8")
            self.bpy.data.filepath = str(scene_path.with_suffix(".blend"))
            package_root = FakeObject(
                "StarBreaker RSI Aurora",
                starbreaker_package_root=True,
                starbreaker_scene_path="Packages/RSI Aurora Mk2_LOD0_TEX0/scene.json",
            )

            resolved = self.package_ops.resolve_package_scene_path(package_root)

            self.assertEqual(resolved, scene_path)
        self.bpy.data.filepath = ""

    def test_apply_paint_to_package_root_restores_base_sidecar_when_leaving_variant(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Scorpius",
            starbreaker_paint_variant_sidecar="variant.materials.json",
            starbreaker_palette_id="palette/skull",
        )
        package_root.type = "EMPTY"
        child = FakeObject(
            "livery_decal_body",
            starbreaker_material_sidecar="variant.materials.json",
            starbreaker_instance_json=json.dumps({"material_sidecar": "base.materials.json"}),
        )
        child.type = "MESH"
        child.parent = package_root
        package_root.children.append(child)

        fake_package = types.SimpleNamespace(
            paints={},
            liveries={"default": types.SimpleNamespace(material_sidecars=["base.materials.json"])},
            scene=types.SimpleNamespace(root_entity=types.SimpleNamespace(material_sidecar="base.materials.json")),
        )

        rebuild_calls: list[tuple[str, str | None, str | None]] = []

        class FakeImporter:
            def __init__(self, context, package, package_root=None):
                self.context = context
                self.package = package
                self.package_root = package_root

            def rebuild_object_materials(self, obj, palette_id):
                rebuild_calls.append((obj.name, obj.get("starbreaker_material_sidecar"), palette_id))
                return 1

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        original_importer = importer_stub.PackageImporter
        original_loader = self.package_ops._load_package_from_root
        original_scene_instance = self.package_ops._scene_instance_from_object
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            importer_stub.PackageImporter = FakeImporter
            self.package_ops._load_package_from_root = lambda _root: fake_package
            self.package_ops._scene_instance_from_object = lambda obj: types.SimpleNamespace(material_sidecar="base.materials.json")
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode

            applied = self.package_ops.apply_paint_to_package_root(
                types.SimpleNamespace(),
                package_root,
                "palette/rsi_scorpius",
            )
        finally:
            importer_stub.PackageImporter = original_importer
            self.package_ops._load_package_from_root = original_loader
            self.package_ops._scene_instance_from_object = original_scene_instance
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(child.get("starbreaker_material_sidecar"), "base.materials.json")
        self.assertNotIn("starbreaker_paint_variant_sidecar", package_root)
        self.assertEqual(rebuild_calls, [("livery_decal_body", "base.materials.json", "palette/rsi_scorpius")])


class AnimationDisplayNameTests(unittest.TestCase):
    """Tests for _animation_display_name and _entity_name_prefix."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.package_ops, _ = _load_package_ops()

    def _clip(self, name: str, **extra) -> dict:
        return {"name": name, **extra}

    def test_localized_name_takes_priority_over_raw_clip_name(self) -> None:
        clip = self._clip("test_ship_vtol_deploy", localized_name="VTOL Deploy")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "VTOL Deploy")

    def test_entity_prefix_stripped_and_humanized(self) -> None:
        clip = self._clip("test_ship_vtol_deploy")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "Vtol Deploy")

    def test_entity_prefix_stripped_retract_variant(self) -> None:
        clip = self._clip("test_ship_vtol_retract")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "Vtol Retract")

    def test_no_entity_prefix_returns_raw_name(self) -> None:
        clip = self._clip("test_ship_vtol_deploy")
        result = self.package_ops._animation_display_name(clip)
        self.assertEqual(result, "test_ship_vtol_deploy")

    def test_prefix_mismatch_still_humanizes(self) -> None:
        # Prefix does not match clip name — entire name is humanized.
        clip = self._clip("other_ship_action")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "Other Ship Action")

    def test_entity_name_prefix_strips_class_namespace(self) -> None:
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(entity_name="EntityClassDefinition.Test_Ship")
            )
        )
        result = self.package_ops._entity_name_prefix(package)
        self.assertEqual(result, "test_ship")

    def test_entity_name_prefix_no_namespace(self) -> None:
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(entity_name="Other_Ship")
            )
        )
        result = self.package_ops._entity_name_prefix(package)
        self.assertEqual(result, "other_ship")

    def test_entity_name_prefix_missing_attribute_returns_empty(self) -> None:
        package = types.SimpleNamespace()
        result = self.package_ops._entity_name_prefix(package)
        self.assertEqual(result, "")

    def test_available_package_animation_items_strips_entity_prefix(self) -> None:
        """Non-fragment clips get entity prefix stripped and humanized."""
        clips = [
            {"name": "test_ship_vtol_deploy", "fps": 30, "frame_count": 60,
             "fragments": [], "sidecar": None, "start_position": None, "start_rotation": None},
            {"name": "test_ship_vtol_retract", "fps": 30, "frame_count": 60,
             "fragments": [], "sidecar": None, "start_position": None, "start_rotation": None},
        ]
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(
                    entity_name="EntityClassDefinition.Test_Ship",
                    raw={"animations": clips},
                )
            )
        )
        items = self.package_ops.available_package_animation_items(package)
        item_map = dict(items)
        self.assertEqual(item_map.get("test_ship_vtol_deploy"), "Vtol Deploy")
        self.assertEqual(item_map.get("test_ship_vtol_retract"), "Vtol Retract")

    def test_available_package_animation_items_frag_tags_override_clip_name(self) -> None:
        """Fragment clips with frag_tags use frag_tags-derived names regardless of prefix."""
        clips = [
            {
                "name": "test_ship_wings_deploy",
                "fps": 30, "frame_count": 120,
                "sidecar": None, "start_position": None, "start_rotation": None,
                "fragments": [{"fragment": "Wings", "frag_tags": ["Retract"],
                               "tags": [], "scopes": [], "animations": [],
                               "guid": "a", "option_weight": 1.0, "blend_out_duration": 0.2}],
            },
            {
                "name": "test_ship_wings_retract",
                "fps": 30, "frame_count": 120,
                "sidecar": None, "start_position": None, "start_rotation": None,
                "fragments": [{"fragment": "Wings", "frag_tags": ["Deploy"],
                               "tags": [], "scopes": [], "animations": [],
                               "guid": "b", "option_weight": 1.0, "blend_out_duration": 0.2}],
            },
        ]
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(
                    entity_name="EntityClassDefinition.Test_Ship",
                    raw={"animations": clips},
                )
            )
        )
        items = self.package_ops.available_package_animation_items(package)
        item_map = dict(items)
        self.assertEqual(item_map.get("fragment:0:test_ship_wings_deploy"), "Wings Retract")
        self.assertEqual(item_map.get("fragment:0:test_ship_wings_retract"), "Wings Deploy")

    def test_animation_insert_label_uses_fragment_display_name(self) -> None:
        clip = {
            "name": "test_ship_wings_retract",
            "fragments": [
                {
                    "fragment": "Wings",
                    "frag_tags": ["Deploy"],
                    "tags": [],
                    "scopes": [],
                    "animations": [],
                }
            ],
        }
        label = self.package_ops._animation_insert_label(
            "fragment:0:test_ship_wings_retract",
            clip,
        )
        self.assertEqual(label, "Wings Deploy")

    def test_animation_insert_label_falls_back_to_animation_key(self) -> None:
        clip = {"name": "test_ship_vtol_deploy", "fragments": []}
        label = self.package_ops._animation_insert_label("test_ship_vtol_deploy", clip)
        self.assertEqual(label, "test_ship_vtol_deploy")


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
