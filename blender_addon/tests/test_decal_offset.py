from __future__ import annotations

import contextlib
import io
import json
from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent
VULTURE_ALT_A = REPO_ROOT / "ships/Data/Objects/Spaceships/Ships/DRAK/Vulture/drak_vulture_alt_a_TEX0.materials.json"

sys.path.insert(0, str(ADDON_ROOT))


if "starbreaker_addon" not in sys.modules:
    package = types.ModuleType("starbreaker_addon")
    package.__path__ = [str(ADDON_ROOT / "starbreaker_addon")]
    sys.modules["starbreaker_addon"] = package

if "starbreaker_addon.runtime" not in sys.modules:
    runtime_package = types.ModuleType("starbreaker_addon.runtime")
    runtime_package.__path__ = [str(ADDON_ROOT / "starbreaker_addon" / "runtime")]
    sys.modules["starbreaker_addon.runtime"] = runtime_package

if "starbreaker_addon.runtime.importer" not in sys.modules:
    importer_package = types.ModuleType("starbreaker_addon.runtime.importer")
    importer_package.__path__ = [str(ADDON_ROOT / "starbreaker_addon" / "runtime" / "importer")]
    sys.modules["starbreaker_addon.runtime.importer"] = importer_package


if "mathutils" not in sys.modules:
    mathutils = types.ModuleType("mathutils")

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

if "numpy" not in sys.modules:
    numpy = types.ModuleType("numpy")
    sys.modules["numpy"] = numpy


from starbreaker_addon.runtime.constants import (
    PROP_DECAL_HOST_CHANNEL,
    PROP_DECAL_HOST_RGB,
    PROP_HAS_POM,
    PROP_MATERIAL_IDENTITY,
    PROP_MATERIAL_SIDECAR,
    PROP_PALETTE_SCOPE,
    PROP_SUBMATERIAL_JSON,
    PROP_TEMPLATE_KEY,
)
from starbreaker_addon.manifest import MaterialSidecar, SubmaterialRecord
from starbreaker_addon.runtime.importer.builders import (
    BuildersMixin,
    _illum_payload_is_local_opacity_decal,
    _mesh_decal_neutral_breakup_default,
    _parallax_height_sampler_extension,
)
from starbreaker_addon.runtime.importer.decals import DecalsMixin
from starbreaker_addon.runtime.importer.materials import MaterialsMixin
from starbreaker_addon.runtime.importer.materials import _material_datablock_is_valid
from starbreaker_addon.runtime.importer.orchestration import OrchestrationMixin
from starbreaker_addon.material_contract import ContractInput, ShaderGroupContract
from starbreaker_addon.runtime.importer.utils import (
    _canonical_material_sidecar_path,
    _material_identity,
    _scene_attachment_offset_to_blender,
)
from starbreaker_addon.templates import template_plan_for_submaterial


class FakeNodeTree:
    def __init__(self):
        self.nodes = []
        self.links = []


class FakeMaterial(dict):
    def __init__(self, name: str, **props):
        library = props.pop("library", None)
        super().__init__(props)
        self.name = name
        self.node_tree = FakeNodeTree()
        self.use_nodes = True
        self.library = library

    def copy(self):
        return FakeMaterial(self.name, library=None, **dict(self))


class FakeLink:
    def __init__(self, from_socket, to_socket):
        self.from_socket = from_socket
        self.to_socket = to_socket


class FakeSocket:
    def __init__(self, name: str, *, is_output: bool):
        self.name = name
        self.is_output = is_output
        self.links: list[FakeLink] = []
        self.default_value = None
        self.node = None


class FakeSocketCollection:
    def __init__(self, sockets: list[FakeSocket]):
        self._sockets = sockets
        self._by_name = {socket.name: socket for socket in sockets}

    def __getitem__(self, index: int):
        return self._sockets[index]

    def __iter__(self):
        return iter(self._sockets)

    def get(self, name: str, default=None):
        return self._by_name.get(name, default)


class FakeLinks(list):
    def new(self, from_socket, to_socket):
        link = FakeLink(from_socket, to_socket)
        self.append(link)
        from_socket.links.append(link)
        to_socket.links.append(link)
        return link

    def remove(self, link):
        if link in self:
            super().remove(link)
        if link in link.from_socket.links:
            link.from_socket.links.remove(link)
        if link in link.to_socket.links:
            link.to_socket.links.remove(link)


class FakeNodeGroupTree:
    def __init__(self, name: str):
        self.name = name


class FakeNode:
    def __init__(
        self,
        bl_idname: str,
        *,
        name: str = "",
        node_tree: FakeNodeGroupTree | None = None,
        inputs: list[str] | None = None,
        outputs: list[str] | None = None,
    ):
        self.bl_idname = bl_idname
        self.name = name
        self.label = ""
        self.node_tree = node_tree
        self.location = types.SimpleNamespace(x=0.0, y=0.0)
        self.blend_type = ""
        self.inputs = FakeSocketCollection([FakeSocket(socket, is_output=False) for socket in (inputs or [])])
        self.outputs = FakeSocketCollection([FakeSocket(socket, is_output=True) for socket in (outputs or [])])
        for socket in [*self.inputs._sockets, *self.outputs._sockets]:
            socket.node = self


class FakeNodes(list):
    def new(self, bl_idname: str):
        if bl_idname == "ShaderNodeRGB":
            node = FakeNode(bl_idname, outputs=["Color"])
        elif bl_idname == "ShaderNodeMixRGB":
            node = FakeNode(bl_idname, inputs=["Fac", "Color1", "Color2"], outputs=["Color"])
        elif bl_idname == "ShaderNodeMath":
            node = FakeNode(bl_idname, inputs=["Value", "Value"], outputs=["Value"])
        elif bl_idname == "ShaderNodeGroup":
            node = FakeNode(bl_idname, inputs=["Base Color", "Base Alpha"], outputs=["Color", "Alpha", "Shader"])
        elif bl_idname == "ShaderNodeTexImage":
            node = FakeNode(bl_idname, inputs=["Vector"], outputs=["Color", "Alpha"])
        else:
            node = FakeNode(bl_idname)
        self.append(node)
        return node

    def get(self, name: str, default=None):
        for node in self:
            if node.name == name:
                return node
        return default


class FakeMaterialNodeTree:
    def __init__(self):
        self.nodes = FakeNodes()
        self.links = FakeLinks()

    def clone(self):
        clone = FakeMaterialNodeTree()
        mapping: dict[int, FakeNode] = {}
        socket_mapping: dict[int, FakeSocket] = {}
        for node in self.nodes:
            copied = FakeNode(
                node.bl_idname,
                name=node.name,
                node_tree=node.node_tree,
                inputs=[socket.name for socket in node.inputs._sockets],
                outputs=[socket.name for socket in node.outputs._sockets],
            )
            copied.label = node.label
            copied.blend_type = node.blend_type
            copied.location = types.SimpleNamespace(x=node.location.x, y=node.location.y)
            clone.nodes.append(copied)
            mapping[id(node)] = copied
            for source, target in zip(node.inputs._sockets, copied.inputs._sockets):
                target.default_value = source.default_value
                socket_mapping[id(source)] = target
            for source, target in zip(node.outputs._sockets, copied.outputs._sockets):
                target.default_value = source.default_value
                socket_mapping[id(source)] = target
        for link in self.links:
            clone.links.new(socket_mapping[id(link.from_socket)], socket_mapping[id(link.to_socket)])
        return clone


class FakeNodeMaterial(FakeMaterial):
    def __init__(self, name: str, **props):
        super().__init__(name, **props)
        self.node_tree = FakeMaterialNodeTree()
        self.blend_method = "BLEND"
        self.use_screen_refraction = True
        self.users = 0

    def copy(self):
        material = FakeNodeMaterial(self.name, **dict(self))
        material.node_tree = self.node_tree.clone()
        return material


class FakeMaterialsCollection(dict):
    def __iter__(self):
        return iter(self.values())

    def get(self, name: str, default=None):
        return super().get(name, default)

    def new(self, name: str):
        material = FakeMaterial(name)
        self[name] = material
        return material


class FakeMatrixWorld:
    def __init__(self, translation: tuple[float, float, float]):
        self._rows = (
            (1.0, 0.0, 0.0, translation[0]),
            (0.0, 1.0, 0.0, translation[1]),
            (0.0, 0.0, 1.0, translation[2]),
            (0.0, 0.0, 0.0, 1.0),
        )

    def __getitem__(self, index: int):
        return self._rows[index]


class FakeSlot:
    def __init__(self, material):
        self.material = material


class FakePolygon:
    def __init__(self, material_index: int, vertices: list[int]):
        self.material_index = material_index
        self.vertices = vertices


class FakeVertex:
    def __init__(self, co: tuple[float, float, float]):
        self.co = types.SimpleNamespace(x=co[0], y=co[1], z=co[2])


class FakeMesh:
    def __init__(
        self,
        polygons: list[FakePolygon],
        vertex_count: int,
        vertices: list[tuple[float, float, float]] | None = None,
    ):
        self.polygons = polygons
        self.vertices = [
            FakeVertex(co)
            for co in (
                vertices
                if vertices is not None
                else [(0.0, 0.0, 0.0) for _index in range(vertex_count)]
            )
        ]
        self.materials = []

    def as_pointer(self) -> int:
        return id(self)


class FakeObject:
    def __init__(self, material_slots: list[FakeSlot], mesh: FakeMesh, **props):
        self.name = props.pop("name", "FakeObject")
        self.type = "MESH"
        self.material_slots = material_slots
        self.data = mesh
        if hasattr(self.data, "materials") and not self.data.materials:
            self.data.materials.extend(slot.material for slot in material_slots)
        self._props = dict(props)

    def get(self, name: str, default=None):
        return self._props.get(name, default)


class ImporterUnderTest(BuildersMixin):
    def __init__(self, *, channel: str | None = None, fallback_rgb: tuple[float, float, float] | None = None):
        self.channel = channel
        self.fallback_rgb = fallback_rgb
        self.illum_rgb_calls: list[tuple[float, float, float]] = []
        self.illum_decal_rgb_calls: list[tuple[float, float, float]] = []
        self.illum_decal_material_calls: list[tuple[str, str]] = []
        self.illum_host_decal_calls: list[tuple[str, str]] = []

    def _mesh_decal_host_channel_for_object(self, obj):
        return self.channel

    def _mesh_decal_host_rgb_for_object(self, obj):
        return self.fallback_rgb

    def _ensure_illum_pom_host_rgb_variant(self, material, rgb):
        self.illum_rgb_calls.append(rgb)
        return FakeMaterial(f"{material.name}__host_rgb", **dict(material))

    def _ensure_illum_decal_host_rgb_variant(self, material, rgb):
        self.illum_decal_rgb_calls.append(rgb)
        return FakeMaterial(f"{material.name}__host_rgb", **dict(material))

    def _ensure_illum_decal_host_material_variant(self, material, host_material):
        self.illum_decal_material_calls.append((material.name, host_material.name))
        return FakeMaterial(f"{material.name}__host_mat", **dict(material))

    def _ensure_host_with_illum_decal_opacity_variant(self, decal_material, host_material):
        self.illum_host_decal_calls.append((host_material.name, decal_material.name))
        return FakeMaterial(f"{host_material.name}__decal_test", **dict(host_material))


class MeshDecalRebindImporterUnderTest(BuildersMixin):
    def __init__(
        self,
        channel: str | None = None,
        authored_rgb: tuple[float, float, float] | None = None,
    ):
        self.channel = channel
        self.authored_rgb = authored_rgb
        self.mesh_variant_calls: list[tuple[str, str]] = []
        self.mesh_rgb_variant_calls: list[tuple[str, tuple[float, float, float]]] = []

    def _mesh_decal_host_channel_for_object(self, obj):
        return self.channel

    def _mesh_decal_host_rgb_for_object(self, obj):
        return None

    def _ensure_mesh_decal_host_variant(self, material, channel, palette):
        self.mesh_variant_calls.append((material.name, channel))
        return FakeMaterial(f"{material.name}__host_{channel}", **dict(material))

    def _ensure_mesh_decal_host_rgb_variant(self, material, rgb):
        self.mesh_rgb_variant_calls.append((material.name, rgb))
        return FakeMaterial(f"{material.name}__host_rgb", **dict(material))

    def _read_paint_tint_rgb(self, _material):
        return self.authored_rgb


class MeshDecalVariantImporterUnderTest(BuildersMixin):
    def _palette_group_node(self, nodes, links, palette, *, x: int, y: int):
        node = FakeNode(
            "ShaderNodeGroup",
            name="Palette",
            node_tree=FakeNodeGroupTree("StarBreaker Palette test"),
            outputs=["Secondary"],
        )
        node.location = types.SimpleNamespace(x=float(x), y=float(y))
        nodes.append(node)
        return node


class HostRoutingImporterUnderTest(BuildersMixin):
    def __init__(self):
        self.host_channel_cache = {}
        self.host_rgb_cache = {}


class FakePackage:
    def __init__(self, has_decal_texture: bool):
        self.has_decal_texture = has_decal_texture

    def resolve_path(self, relative_path):
        if self.has_decal_texture and relative_path:
            return Path("/tmp") / Path(relative_path).name
        return None


class DecalDefaultsImporterUnderTest(DecalsMixin):
    def __init__(self, has_decal_texture: bool):
        self.package = FakePackage(has_decal_texture)


class MaterialReuseImporterUnderTest(MaterialsMixin):
    def __init__(self):
        self.material_cache = {}
        self.material_identity_index = {}
        self.material_identity_index_ready = False
        self.package = None
        self.package_root = None
        self.rebuild_calls: list[str] = []

    def _palette_scope(self, palette=None) -> str:
        return "test-scope"

    def _ensure_material_identity_index(self) -> None:
        self.material_identity_index_ready = True

    def _build_managed_material(
        self,
        material,
        sidecar_path,
        sidecar,
        submaterial,
        palette,
        material_identity,
    ) -> None:
        self.rebuild_calls.append(material.name)
        material[PROP_TEMPLATE_KEY] = template_plan_for_submaterial(submaterial).template_key
        material[PROP_MATERIAL_IDENTITY] = material_identity
        material[PROP_MATERIAL_SIDECAR] = _canonical_material_sidecar_path(sidecar_path, sidecar)
        material[PROP_SUBMATERIAL_JSON] = json.dumps(submaterial.raw, sort_keys=True)
        material[PROP_PALETTE_SCOPE] = self._palette_scope(palette)


class ManagedMaterialBuildImporterUnderTest(BuildersMixin):
    def __init__(self):
        self.build_calls: list[str] = []

    def _build_nodraw_material(self, material) -> None:
        self.build_calls.append("nodraw")

    def _build_illum_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("illum")

    def _build_hard_surface_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("hard_surface")

    def _group_contract_for_submaterial(self, submaterial):
        return None

    def _build_contract_group_material(self, material, submaterial, palette, plan, group_contract) -> bool:
        return False

    def _build_glass_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("glass")

    def _build_screen_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("screen")

    def _build_effect_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("effects")

    def _build_principled_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("principled")

    def _apply_material_node_layout(self, material) -> None:
        return None

    def _sweep_unreachable_nodes(self, material) -> None:
        return None

    def _palette_scope(self, palette=None) -> str:
        return "test-scope"


class FakeSidecar:
    def __init__(self, submaterials):
        self.submaterials = submaterials


class FakeSubmaterial:
    def __init__(self, index: int, name: str):
        self.index = index
        self.submaterial_name = name


class TestMeshDecalNeutralBreakupDefault(unittest.TestCase):
    def test_returns_white_for_stencil_mesh_decal_without_breakup_texture(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {"has_stencil_map": True, "tokens": ["STENCIL_MAP"]},
            }
        )
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )
        contract_input = ContractInput(
            name="TexSlot8_GrimeBreakup",
            socket_type="NodeSocketColor",
            semantic="grime_breakup",
            source_slot="TexSlot8",
            required=False,
            default_value=None,
            raw={},
        )

        self.assertEqual(
            _mesh_decal_neutral_breakup_default(
                group_contract,
                submaterial,
                contract_input,
                source_socket=None,
            ),
            (1.0, 1.0, 1.0, 1.0),
        )

    def test_returns_white_for_pom_mesh_decal_without_breakup_texture(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "has_parallax_occlusion_mapping": True,
                    "tokens": ["PARALLAX_OCCLUSION_MAPPING"],
                },
            }
        )
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )
        contract_input = ContractInput(
            name="TexSlot8_GrimeBreakup",
            socket_type="NodeSocketColor",
            semantic="grime_breakup",
            source_slot="TexSlot8",
            required=False,
            default_value=None,
            raw={},
        )

        self.assertEqual(
            _mesh_decal_neutral_breakup_default(
                group_contract,
                submaterial,
                contract_input,
                source_socket=None,
            ),
            (1.0, 1.0, 1.0, 1.0),
        )

    def test_returns_none_when_breakup_texture_is_present(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {"has_stencil_map": True, "tokens": ["STENCIL_MAP"]},
            }
        )
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )
        contract_input = ContractInput(
            name="TexSlot8_GrimeBreakup",
            socket_type="NodeSocketColor",
            semantic="grime_breakup",
            source_slot="TexSlot8",
            required=False,
            default_value=None,
            raw={},
        )

        self.assertIsNone(
            _mesh_decal_neutral_breakup_default(
                group_contract,
                submaterial,
                contract_input,
                source_socket=object(),
            )
        )


class FakePackageWithSidecars:
    def __init__(self, sidecar):
        self.sidecar = sidecar
        self.palettes = {}
        self.scene = types.SimpleNamespace(root_entity=types.SimpleNamespace(palette_id=None))

    def load_material_sidecar(self, sidecar_path):
        return self.sidecar


class OrchestrationImporterUnderTest(OrchestrationMixin, BuildersMixin):
    def __init__(self, sidecar):
        self.package = FakePackageWithSidecars(sidecar)
        self.package_root = None
        self.import_palette_override = None
        self.import_paint_variant_sidecar = None
        self.exterior_material_sidecars = None
        self.mesh_polygon_counts_cache = {}
        self.slot_mapping_cache = {}
        self.sidecar_submaterials_by_index = {}
        self.sidecar_submaterials_by_name = {}
        self.sidecar_submaterials_by_name_all = {}

    def _ensure_runtime_shared_groups(self) -> None:
        return None

    def _effective_palette_id(self, palette_id: str | None) -> str | None:
        return palette_id

    def material_for_submaterial(self, sidecar_path, sidecar, submaterial, palette):
        return FakeMaterial(f"material_{submaterial.index}")

    def _remove_replaced_slot_material(self, material) -> None:
        return None

    def _rebind_mesh_decal_for_host(self, obj, palette, **_kwargs) -> int:
        return 0


class OrphanRemovalImporterUnderTest(OrchestrationMixin):
    def __init__(self):
        self._pending_orphan_materials = set()


class FakeHashableMaterial:
    library = None

    def __init__(self):
        self.users = 0

    def get(self, _key, default=None):
        return default


class FakeInvalidMaterial:
    @property
    def name(self):
        raise ReferenceError("StructRNA of type Material has been removed")


class DecalOffsetTests(unittest.TestCase):
    def test_invalid_material_datablock_is_detected(self) -> None:
        self.assertFalse(_material_datablock_is_valid(FakeInvalidMaterial()))

    def test_replaced_slot_material_cleanup_is_deferred(self) -> None:
        importer = OrphanRemovalImporterUnderTest()
        material = FakeHashableMaterial()

        importer._remove_replaced_slot_material(material)

        self.assertIn(material, importer._pending_orphan_materials)

    def test_rebuild_object_materials_skips_empty_unmapped_slots_without_warning(self) -> None:
        sidecar = FakeSidecar([
            FakeSubmaterial(0, "decal pom"),
            FakeSubmaterial(1, "tint_secondary"),
        ])
        importer = OrchestrationImporterUnderTest(sidecar)
        mesh = FakeMesh(polygons=[], vertex_count=0)
        importer.slot_mapping_cache[mesh.as_pointer()] = [0, 1, None, None]
        obj = FakeObject(
            material_slots=[FakeSlot(None), FakeSlot(None), FakeSlot(None), FakeSlot(None)],
            mesh=mesh,
            name="flair_poster_hook_mesh_005",
            starbreaker_material_sidecar="Data/Objects/props/flair/poster/flair_poster_1_a_TEX0.materials.json",
        )

        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            applied = importer.rebuild_object_materials(obj, None)

        self.assertEqual(applied, 2)
        self.assertEqual(output.getvalue(), "")
        self.assertIsNotNone(obj.material_slots[0].material)
        self.assertIsNotNone(obj.material_slots[1].material)
        self.assertIsNone(obj.material_slots[2].material)
        self.assertIsNone(obj.material_slots[3].material)

    def test_illum_pom_rebind_keeps_original_material_with_palette_channel(self) -> None:
        decal = FakeMaterial(
            "drak_vulture:pom_decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(decal)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = ImporterUnderTest(channel="primary", fallback_rgb=None)

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.illum_rgb_calls, [])
        self.assertIs(obj.material_slots[0].material, decal)

    def test_illum_pom_rebind_keeps_original_material_with_fallback_rgb(self) -> None:
        decal = FakeMaterial(
            "drak_vulture:pom_decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(decal)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        palette = types.SimpleNamespace(
            primary=(0.85, 0.72, 0.12),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = ImporterUnderTest(
            channel="primary",
            fallback_rgb=(0.0627, 0.0627, 0.0627),
        )

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.illum_rgb_calls, [])
        self.assertIs(obj.material_slots[0].material, decal)

    def test_illum_decal_opacity_without_spatial_host_does_not_use_rgb_fallback(self) -> None:
        decal = FakeMaterial(
            "optc_s03_behr_02_mat:decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: False,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "Illum",
                        "decoded_feature_flags": {
                            "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                            "has_decal": True,
                        }
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(decal)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        importer = ImporterUnderTest(channel=None, fallback_rgb=(0.02, 0.03, 0.04))

        rebound = importer._rebind_mesh_decal_for_host(obj, None)

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.illum_rgb_calls, [])
        self.assertEqual(importer.illum_decal_rgb_calls, [])
        self.assertIs(obj.material_slots[0].material, decal)

    def test_illum_decal_opacity_payload_uses_local_texture_semantics(self) -> None:
        payload = {
            "shader_family": "Illum",
            "decoded_feature_flags": {
                "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                "has_decal": True,
                "has_parallax_occlusion_mapping": False,
            },
            "texture_slots": [
                {"slot": "TexSlot1", "role": "base_color", "is_virtual": False},
                {"slot": "TexSlot2", "role": "normal_gloss", "is_virtual": False},
            ],
            "virtual_inputs": [],
        }

        self.assertTrue(_illum_payload_is_local_opacity_decal(payload))

    def test_illum_decal_opacity_payload_rejects_virtual_tint_palette_source(self) -> None:
        payload = {
            "shader_family": "Illum",
            "decoded_feature_flags": {
                "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                "has_decal": True,
                "has_parallax_occlusion_mapping": False,
            },
            "texture_slots": [
                {"slot": "TexSlot7", "role": "tint_palette_decal", "is_virtual": True},
            ],
            "virtual_inputs": ["$TintPaletteDecal"],
        }

        self.assertFalse(_illum_payload_is_local_opacity_decal(payload))

    def test_illum_decal_host_rgb_variant_keeps_decal_texture_at_illum_input(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs[0], layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs[1], layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs[0], illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs[1], illum.inputs.get("Primary Alpha"))

            variant = BuildersMixin()._ensure_illum_decal_host_rgb_variant(decal, (0.1, 0.2, 0.3))

            variant_illum = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "StarBreaker Runtime Illum"
            )
            self.assertEqual(variant.get("starbreaker_decal_host_composite_mode"), "illum_primary_color_v2")
            primary_color = variant_illum.inputs.get("Primary Color")
            primary_alpha = variant_illum.inputs.get("Primary Alpha")
            self.assertEqual(primary_alpha.links, [])
            self.assertEqual(primary_alpha.default_value, 1.0)
            self.assertEqual(primary_color.links[0].from_socket.name, "Color")
            self.assertEqual(primary_color.links[0].from_socket.node.name, "SB_DecalHostCompositeMix")
            mix = primary_color.links[0].from_socket.node
            self.assertEqual(mix.inputs[2].links[0].from_socket.node.name, "Layer Surface")
            self.assertEqual(mix.inputs[2].links[0].from_socket.name, "Color")
            self.assertEqual(variant.blend_method, "OPAQUE")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_illum_opacity_nearest_host_rebind_uses_host_material_overlay(self) -> None:
        decal = FakeMaterial(
            "optc_s03_behr_02_mat:decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: False,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "Illum",
                        "decoded_feature_flags": {
                            "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                            "has_decal": True,
                        }
                    }
                ),
            },
        )
        host = FakeMaterial(
            "optc_s03_behr_02_mat:H_Paint_01_B",
            starbreaker_shader_family="Illum",
        )
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(0, [3, 4, 5]),
            ],
            vertex_count=6,
            vertices=[
                (0.0, 0.0, 0.0),
                (1.0, 0.0, 0.0),
                (0.0, 1.0, 0.0),
                (0.1, 0.1, 0.0),
                (1.1, 0.1, 0.0),
                (0.1, 1.1, 0.0),
            ],
        )
        obj = FakeObject(material_slots=[FakeSlot(decal), FakeSlot(host)], mesh=mesh)
        importer = ImporterUnderTest(channel=None, fallback_rgb=(0.02, 0.03, 0.04))

        rebound = importer._rebind_illum_opacity_decals_by_nearest_host(obj, None, [(0, decal)])

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.illum_host_decal_calls, [(host.name, decal.name)])
        self.assertEqual(importer.illum_decal_material_calls, [])
        self.assertEqual(importer.illum_decal_rgb_calls, [])
        self.assertEqual(mesh.polygons[1].material_index, 2)
        self.assertEqual(mesh.materials[2].name, "optc_s03_behr_02_mat:H_Paint_01_B__decal_test")

    def test_host_with_illum_decal_variant_overlays_decal_on_host_color(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs.get("Color"), layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs.get("Alpha"), layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs.get("Color"), illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs.get("Alpha"), illum.inputs.get("Primary Alpha"))

            host = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B")
            host_layered = FakeNode(
                "ShaderNodeGroup",
                name="Host LayeredInputs",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayeredInputs"),
                outputs=["Color"],
            )
            host_principled = FakeNode(
                "ShaderNodeGroup",
                name="Host Principled",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Principled"),
                inputs=["Base Color", "Alpha", "Normal Color"],
                outputs=["Shader"],
            )
            host.node_tree.nodes.extend([host_layered, host_principled])
            host.node_tree.links.new(host_layered.outputs.get("Color"), host_principled.inputs.get("Base Color"))

            variant = BuildersMixin()._ensure_host_with_illum_decal_opacity_variant(decal, host)

            self.assertTrue(variant.name.startswith("optc_s03_behr_02_mat:H_Paint_01_B__decal_"))
            self.assertNotIn("__host_mat_", variant.name)
            self.assertEqual(variant.get("starbreaker_illum_decal_composite_mode"), "host_with_illum_decal_opacity_v2")
            variant_principled = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "StarBreaker Runtime Principled"
            )
            base_color = variant_principled.inputs.get("Base Color")
            mix = base_color.links[0].from_socket.node
            self.assertEqual(mix.name, "SB_IllumDecalOverlayColor")
            self.assertEqual(mix.inputs[0].links[0].from_socket.name, "Alpha")
            self.assertEqual(mix.inputs[1].links[0].from_socket.node.name, "Host LayeredInputs")
            self.assertEqual(mix.inputs[2].links[0].from_socket.node.name, "SB_Decal_Layer Surface")
            self.assertEqual(variant.blend_method, "OPAQUE")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_host_with_illum_decal_variant_multiplies_decal_public_opacity(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial(
                "optc_s03_behr_02_mat:decals",
                **{
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "Illum",
                            "decoded_feature_flags": {
                                "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                                "has_decal": True,
                            },
                            "public_params": {
                                "DecalDiffuseOpacity": 0.5,
                                "DecalAlphaMult": 0.25,
                            },
                        }
                    )
                },
            )
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs.get("Color"), layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs.get("Alpha"), layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs.get("Color"), illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs.get("Alpha"), illum.inputs.get("Primary Alpha"))

            host = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B")
            host_layered = FakeNode(
                "ShaderNodeGroup",
                name="Host LayeredInputs",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayeredInputs"),
                outputs=["Color"],
            )
            host_principled = FakeNode(
                "ShaderNodeGroup",
                name="Host Principled",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Principled"),
                inputs=["Base Color", "Alpha"],
                outputs=["Shader"],
            )
            host.node_tree.nodes.extend([host_layered, host_principled])
            host.node_tree.links.new(host_layered.outputs.get("Color"), host_principled.inputs.get("Base Color"))

            variant = BuildersMixin()._ensure_host_with_illum_decal_opacity_variant(decal, host)

            alpha_mul = next(node for node in variant.node_tree.nodes if node.name == "SB_IllumDecalOverlayAlpha")
            self.assertEqual(alpha_mul.operation, "MULTIPLY")
            self.assertTrue(alpha_mul.use_clamp)
            self.assertAlmostEqual(alpha_mul.inputs[1].default_value, 0.125)
            variant_principled = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "StarBreaker Runtime Principled"
            )
            self.assertEqual(variant_principled.inputs.get("Alpha").links, [])
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_host_with_illum_decal_variant_does_not_reuse_stale_v1_clone(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs.get("Color"), layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs.get("Alpha"), layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs.get("Color"), illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs.get("Alpha"), illum.inputs.get("Primary Alpha"))

            host = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B")
            host_layered = FakeNode(
                "ShaderNodeGroup",
                name="Host LayeredInputs",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayeredInputs"),
                outputs=["Color"],
            )
            host_principled = FakeNode(
                "ShaderNodeGroup",
                name="Host Principled",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Principled"),
                inputs=["Base Color"],
                outputs=["Shader"],
            )
            host.node_tree.nodes.extend([host_layered, host_principled])
            host.node_tree.links.new(host_layered.outputs.get("Color"), host_principled.inputs.get("Base Color"))

            old = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B__decal_deadbeef")
            old["starbreaker_illum_decal_composite_mode"] = "host_with_illum_decal_opacity_v1"
            bpy.data.materials[old.name] = old

            variant = BuildersMixin()._ensure_host_with_illum_decal_opacity_variant(decal, host)

            self.assertIsNot(variant, old)
            self.assertEqual(variant.get("starbreaker_illum_decal_composite_mode"), "host_with_illum_decal_opacity_v2")
            self.assertNotEqual(variant.name, old.name)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_pom_receives_host_tint_variant(self) -> None:
        pom_control = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                        ],
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_control)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = MeshDecalRebindImporterUnderTest(channel="secondary")

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.mesh_variant_calls, [("bsnp_fps_behr_p6lr_mat:poms", "secondary")])
        self.assertEqual(obj.material_slots[0].material.name, "bsnp_fps_behr_p6lr_mat:poms__host_secondary")

    def test_control_only_mesh_decal_pom_ignores_object_level_rgb_fallback(self) -> None:
        pom_control = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                        ],
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_control)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        importer = MeshDecalRebindImporterUnderTest(channel=None)

        rebound = importer._rebind_mesh_decal_for_host(
            obj,
            None,
            fallback_rgb=(0.023, 0.023, 0.023),
        )

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.mesh_variant_calls, [])
        self.assertEqual(importer.mesh_rgb_variant_calls, [])
        self.assertIs(obj.material_slots[0].material, pom_control)

    def test_control_only_mesh_decal_host_variant_restores_relief_alpha(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            decal_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=[
                    "TexSlot1_DecalSource",
                    "TexSlot1_DecalSource_alpha",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                    "Host Tint",
                ],
                outputs=["Shader"],
            )
            alpha_source = FakeNode("ShaderNodeTexImage", name="POM Mask", outputs=["Color", "Alpha"])
            pom_control.node_tree.nodes.append(alpha_source)
            pom_control.node_tree.links.new(alpha_source.outputs.get("Alpha"), decal_group.inputs.get("TexSlot1_DecalSource_alpha"))
            decal_group.inputs.get("TexSlot1_DecalSource_alpha").default_value = 0.0
            decal_group.inputs.get("Param_DecalDiffuseOpacity").default_value = 0.0
            decal_group.inputs.get("Param_DecalAlphaMultiplier").default_value = 0.0
            pom_control.node_tree.nodes.append(decal_group)
            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_variant(
                pom_control,
                "secondary",
                palette,
            )

            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            self.assertEqual(variant.name, "bsnp_fps_behr_p6lr_mat:poms__host_secondary")
            self.assertEqual(variant.get("starbreaker_mesh_decal_variant_mode"), "control_only_pom_masked_v2")
            self.assertEqual(variant_group.inputs.get("TexSlot1_DecalSource").default_value, (1.0, 1.0, 1.0, 1.0))
            alpha = variant_group.inputs.get("TexSlot1_DecalSource_alpha")
            self.assertEqual(alpha.default_value, 0.0)
            self.assertEqual(alpha.links[0].from_socket.node.name, "POM Mask")
            self.assertEqual(alpha.links[0].from_socket.name, "Alpha")
            self.assertEqual(variant_group.inputs.get("Param_DecalDiffuseOpacity").default_value, 1.0)
            self.assertEqual(variant_group.inputs.get("Param_DecalAlphaMultiplier").default_value, 1.0)
            host_tint = variant_group.inputs.get("Host Tint")
            self.assertEqual(host_tint.links[0].from_socket.node.name, "Palette")
            self.assertEqual(host_tint.links[0].from_socket.name, "Secondary")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_mesh_pom_decal_polygons_rebind_to_nearest_host_material(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            poms = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            primary_host = FakeMaterial(
                "test:H_Paint_01_B",
                starbreaker_shader_family="LayerBlend",
                **{
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {"palette_routing": {"material_channel": {"name": "primary"}}}
                    )
                },
            )
            secondary_host = FakeMaterial(
                "test:H_Parkerized_01_B",
                starbreaker_shader_family="LayerBlend",
                **{
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {"palette_routing": {"material_channel": {"name": "secondary"}}}
                    )
                },
            )
            mesh = FakeMesh(
                polygons=[
                    FakePolygon(1, [0, 1, 2]),
                    FakePolygon(2, [3, 4, 5]),
                    FakePolygon(0, [6, 7, 8]),
                    FakePolygon(0, [9, 10, 11]),
                ],
                vertex_count=12,
                vertices=[
                    (0.0, 0.0, 0.0),
                    (0.2, 0.0, 0.0),
                    (0.0, 0.2, 0.0),
                    (10.0, 0.0, 0.0),
                    (10.2, 0.0, 0.0),
                    (10.0, 0.2, 0.0),
                    (0.05, 0.05, 0.05),
                    (0.15, 0.05, 0.05),
                    (0.05, 0.15, 0.05),
                    (10.05, 0.05, 0.05),
                    (10.15, 0.05, 0.05),
                    (10.05, 0.15, 0.05),
                ],
            )
            obj = FakeObject(
                material_slots=[FakeSlot(poms), FakeSlot(primary_host), FakeSlot(secondary_host)],
                mesh=mesh,
            )
            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )
            importer = MeshDecalRebindImporterUnderTest(channel="secondary")

            rebound = importer._rebind_mesh_decal_for_host(obj, palette)

            self.assertEqual(rebound, 2)
            self.assertEqual(
                importer.mesh_variant_calls,
                [
                    ("bsnp_fps_behr_p6lr_mat:poms", "primary"),
                    ("bsnp_fps_behr_p6lr_mat:poms", "secondary"),
                ],
            )
            self.assertEqual(mesh.polygons[2].material_index, 3)
            self.assertEqual(mesh.polygons[3].material_index, 4)
            self.assertEqual(mesh.materials[3].name, "bsnp_fps_behr_p6lr_mat:poms__host_primary")
            self.assertEqual(mesh.materials[4].name, "bsnp_fps_behr_p6lr_mat:poms__host_secondary")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_pom_spatial_rebind_does_not_create_rgb_variant(self) -> None:
        poms = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                        ],
                    }
                ),
            },
        )
        host = FakeMaterial("test:H_Paint_01_B", starbreaker_shader_family="LayerBlend")
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(0, [3, 4, 5]),
            ],
            vertex_count=6,
            vertices=[
                (0.0, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.0, 0.2, 0.0),
                (0.05, 0.05, 0.05),
                (0.15, 0.05, 0.05),
                (0.05, 0.15, 0.05),
            ],
        )
        obj = FakeObject(material_slots=[FakeSlot(poms), FakeSlot(host)], mesh=mesh)
        importer = MeshDecalRebindImporterUnderTest(channel=None, authored_rgb=(0.023, 0.023, 0.023))

        rebound = importer._rebind_mesh_decal_for_host(obj, None)

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.mesh_variant_calls, [])
        self.assertEqual(importer.mesh_rgb_variant_calls, [])
        self.assertEqual(mesh.polygons[1].material_index, 0)
        self.assertEqual(len(mesh.materials), 2)

    def test_visible_mesh_decal_pom_still_receives_host_tint(self) -> None:
        pom_decal = FakeMaterial(
            "drak_clipper_ext:Decal_POM_A",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DECAL", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": True,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                            {"slot": "TexSlot6", "role": "tint_mask"},
                        ],
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_decal)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = MeshDecalRebindImporterUnderTest(channel="secondary")

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.mesh_variant_calls, [("drak_clipper_ext:Decal_POM_A", "secondary")])
        self.assertEqual(obj.material_slots[0].material.name, "drak_clipper_ext:Decal_POM_A__host_secondary")

    def test_mesh_decal_host_variant_names_do_not_chain_existing_host_suffixes(self) -> None:
        bpy = sys.modules["bpy"]
        original_materials = getattr(bpy.data, "materials", None)
        materials = FakeMaterialsCollection()
        bpy.data.materials = materials
        try:
            decal = FakeMaterial(
                "drak_clipper_ext:Decal_POM_A#03e817cc__host_secondary",
                starbreaker_shader_family="MeshDecal",
                **{PROP_HAS_POM: True},
            )
            importer = ImporterUnderTest(channel="tertiary")
            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )

            variant = importer._ensure_mesh_decal_host_variant(decal, "tertiary", palette)

            self.assertEqual(
                variant.name,
                "drak_clipper_ext:Decal_POM_A#03e817cc__host_tertiary",
            )
        finally:
            bpy.data.materials = original_materials

    def test_host_channel_scan_returns_single_channel_without_polygon_counts(self) -> None:
        obj = FakeObject(
            material_slots=[FakeSlot(FakeMaterial("drak_clipper_ext_Paint_Secondary"))],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        importer = ImporterUnderTest()

        self.assertEqual(importer._scan_slots_for_host_channel(obj), "secondary")

    def test_host_channel_scan_uses_polygon_weight_when_channels_compete(self) -> None:
        obj = FakeObject(
            material_slots=[
                FakeSlot(FakeMaterial("drak_clipper_ext_Paint_Primary")),
                FakeSlot(FakeMaterial("drak_clipper_ext_Paint_Tertiary")),
            ],
            mesh=FakeMesh(
                polygons=[
                    FakePolygon(0, [0, 1, 2]),
                    FakePolygon(1, [3, 4, 5]),
                    FakePolygon(1, [6, 7, 8]),
                ],
                vertex_count=9,
            ),
        )
        importer = ImporterUnderTest()

        self.assertEqual(importer._scan_slots_for_host_channel(obj), "tertiary")

    def test_mesh_decal_host_channel_prefers_precomputed_object_property(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            **{PROP_DECAL_HOST_CHANNEL: "glass"},
        )
        importer = HostRoutingImporterUnderTest()

        self.assertEqual(importer._mesh_decal_host_channel_for_object(obj), "glass")

    def test_mesh_decal_host_channel_falls_back_to_parent_precomputed_property(self) -> None:
        parent = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            **{PROP_DECAL_HOST_CHANNEL: "secondary"},
        )
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        obj.parent = parent
        importer = HostRoutingImporterUnderTest()

        self.assertEqual(importer._mesh_decal_host_channel_for_object(obj), "secondary")

    def test_mesh_decal_host_rgb_prefers_precomputed_object_property(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            **{PROP_DECAL_HOST_RGB: [0.2, 0.4, 0.6]},
        )
        importer = HostRoutingImporterUnderTest()

        self.assertEqual(importer._mesh_decal_host_rgb_for_object(obj), (0.2, 0.4, 0.6))

    def test_derive_decal_host_route_from_submaterials_uses_polygon_weight(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(
                polygons=[
                    FakePolygon(0, [0, 1, 2]),
                    FakePolygon(1, [3, 4, 5]),
                    FakePolygon(1, [6, 7, 8]),
                ],
                vertex_count=9,
            ),
        )
        importer = HostRoutingImporterUnderTest()
        slot_submaterials = [
            SubmaterialRecord.from_value(
                {
                    "shader_family": "HardSurface",
                    "palette_routing": {"material_channel": {"name": "primary"}},
                }
            ),
            SubmaterialRecord.from_value(
                {
                    "shader_family": "HardSurface",
                    "palette_routing": {"material_channel": {"name": "tertiary"}},
                }
            ),
        ]

        channel, rgb = importer._derive_decal_host_route_from_submaterials(obj, slot_submaterials)

        self.assertEqual(channel, "tertiary")
        self.assertIsNone(rgb)

    def test_derive_decal_host_route_from_submaterials_falls_back_to_authored_tint(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[FakePolygon(0, [0, 1, 2])], vertex_count=3),
        )
        importer = HostRoutingImporterUnderTest()
        slot_submaterials = [
            SubmaterialRecord.from_value(
                {
                    "shader_family": "HardSurface",
                    "layer_manifest": [
                        {
                            "tint_color": [0.2, 0.4, 0.6],
                        }
                    ],
                }
            )
        ]

        channel, rgb = importer._derive_decal_host_route_from_submaterials(obj, slot_submaterials)

        self.assertIsNone(channel)
        self.assertEqual(rgb, (0.2, 0.4, 0.6))

    def test_parallax_bias_value_prefers_authored_height_bias(self) -> None:
        importer = ImporterUnderTest()
        submaterial = SubmaterialRecord.from_value(
            {
                "public_params": {
                    "HeightBias": 0.75,
                    "PomDisplacement": 0.04,
                }
            }
        )

        self.assertAlmostEqual(importer._parallax_bias_value(submaterial), 0.75)

    def test_parallax_height_sampler_extension_clips_default_uv_range(self) -> None:
        self.assertEqual(_parallax_height_sampler_extension(1.0), "CLIP")
        self.assertEqual(_parallax_height_sampler_extension(0.75), "CLIP")

    def test_parallax_height_sampler_extension_repeats_explicit_tiling(self) -> None:
        self.assertEqual(_parallax_height_sampler_extension(3.0), "REPEAT")

    def test_missing_mesh_decal_texture_defaults_alpha_to_zero(self) -> None:
        submaterial = SubmaterialRecord.from_value({"shader_family": "MeshDecal"})
        palette = types.SimpleNamespace(decal_texture=None)
        importer = DecalDefaultsImporterUnderTest(has_decal_texture=False)

        _, alpha = importer._virtual_tint_palette_decal_defaults(
            submaterial,
            palette,
            has_decal_texture=importer._has_palette_decal_texture(palette),
        )

        self.assertEqual(alpha, 0.0)

    def test_missing_stencil_map_texture_defaults_alpha_to_zero(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "LayerBlend_V2",
                "decoded_feature_flags": {"has_stencil_map": True},
            }
        )
        palette = types.SimpleNamespace(decal_texture=None)
        importer = DecalDefaultsImporterUnderTest(has_decal_texture=False)

        _, alpha = importer._virtual_tint_palette_decal_defaults(
            submaterial,
            palette,
            has_decal_texture=importer._has_palette_decal_texture(palette),
        )

        self.assertEqual(alpha, 0.0)

    def test_mesh_decal_with_texture_keeps_existing_default_alpha(self) -> None:
        submaterial = SubmaterialRecord.from_value({"shader_family": "MeshDecal"})
        palette = types.SimpleNamespace(decal_texture="Data/Textures/paint/decal.png")
        importer = DecalDefaultsImporterUnderTest(has_decal_texture=True)

        _, alpha = importer._virtual_tint_palette_decal_defaults(
            submaterial,
            palette,
            has_decal_texture=importer._has_palette_decal_texture(palette),
        )

        self.assertEqual(alpha, 0.85)


class MaterialReuseTests(unittest.TestCase):
    @unittest.skipUnless(
        VULTURE_ALT_A.is_file(),
        "Vulture fixtures not present; skipping material reuse regression test",
    )
    def test_stale_template_key_forces_managed_material_rebuild(self) -> None:
        sidecar = MaterialSidecar.from_file(VULTURE_ALT_A)
        submaterial = next(
            candidate
            for candidate in sidecar.submaterials
            if candidate.submaterial_name == "livery_decal"
        )
        self.assertEqual(template_plan_for_submaterial(submaterial).template_key, "nodraw")

        bpy = sys.modules["bpy"]
        original_materials = getattr(bpy.data, "materials", None)
        materials = FakeMaterialsCollection()
        bpy.data.materials = materials
        try:
            sidecar_path = _canonical_material_sidecar_path("", sidecar)
            palette_scope = "test-scope"
            material_identity = _material_identity(sidecar_path, sidecar, submaterial, None, palette_scope)
            stale = FakeMaterial(
                submaterial.blender_material_name or "DRAK_Vulture:livery_decal",
                **{
                    PROP_TEMPLATE_KEY: "physical_surface",
                    PROP_MATERIAL_IDENTITY: material_identity,
                    PROP_MATERIAL_SIDECAR: sidecar_path,
                    PROP_SUBMATERIAL_JSON: json.dumps(submaterial.raw, sort_keys=True),
                    PROP_PALETTE_SCOPE: palette_scope,
                },
            )
            materials[stale.name] = stale

            importer = MaterialReuseImporterUnderTest()
            material = importer.material_for_submaterial(sidecar_path, sidecar, submaterial, None)

            self.assertIs(material, stale)
            self.assertEqual(importer.rebuild_calls, [stale.name])
            self.assertEqual(material[PROP_TEMPLATE_KEY], "nodraw")
        finally:
            bpy.data.materials = original_materials

    @unittest.skipUnless(
        VULTURE_ALT_A.is_file(),
        "Vulture fixtures not present; skipping material localization regression test",
    )
    def test_linked_reusable_material_is_copied_before_rebuild(self) -> None:
        sidecar = MaterialSidecar.from_file(VULTURE_ALT_A)
        submaterial = next(
            candidate
            for candidate in sidecar.submaterials
            if candidate.submaterial_name == "livery_decal"
        )

        bpy = sys.modules["bpy"]
        original_materials = getattr(bpy.data, "materials", None)
        materials = FakeMaterialsCollection()
        bpy.data.materials = materials
        try:
            sidecar_path = _canonical_material_sidecar_path("", sidecar)
            palette_scope = "test-scope"
            material_identity = _material_identity(sidecar_path, sidecar, submaterial, None, palette_scope)
            linked = FakeMaterial(
                submaterial.blender_material_name or "DRAK_Vulture:livery_decal",
                library=object(),
                **{
                    PROP_TEMPLATE_KEY: "wrong-template",
                    PROP_MATERIAL_IDENTITY: material_identity,
                    PROP_MATERIAL_SIDECAR: sidecar_path,
                    PROP_SUBMATERIAL_JSON: json.dumps(submaterial.raw, sort_keys=True),
                    PROP_PALETTE_SCOPE: palette_scope,
                },
            )
            materials[linked.name] = linked

            importer = MaterialReuseImporterUnderTest()
            material = importer.material_for_submaterial(sidecar_path, sidecar, submaterial, None)
            second = importer.material_for_submaterial(sidecar_path, sidecar, submaterial, None)

            self.assertIsNot(material, linked)
            self.assertIs(second, material)
            self.assertIsNone(material.library)
            self.assertEqual(material[PROP_MATERIAL_IDENTITY], material_identity)
            self.assertEqual(importer.rebuild_calls, [material.name])
        finally:
            bpy.data.materials = original_materials

    @unittest.skipUnless(
        VULTURE_ALT_A.is_file(),
        "Vulture fixtures not present; skipping managed material dispatch regression test",
    )
    def test_illum_nodraw_submaterial_uses_nodraw_builder(self) -> None:
        sidecar = MaterialSidecar.from_file(VULTURE_ALT_A)
        submaterial = next(
            candidate
            for candidate in sidecar.submaterials
            if candidate.submaterial_name == "livery_decal"
        )
        importer = ManagedMaterialBuildImporterUnderTest()
        material = FakeMaterial(submaterial.blender_material_name or "DRAK_Vulture:livery_decal")

        importer._build_managed_material(
            material,
            _canonical_material_sidecar_path("", sidecar),
            sidecar,
            submaterial,
            None,
            "identity",
        )

        self.assertEqual(importer.build_calls, ["nodraw"])
        self.assertEqual(material[PROP_TEMPLATE_KEY], "nodraw")


class SceneAttachmentOffsetTests(unittest.TestCase):
    def test_duplicate_no_rotation_helper_offset_is_suppressed(self) -> None:
        location = _scene_attachment_offset_to_blender(
            (0.0, -1.2599999904632568, 3.371000051498413),
            (0.0, 0.0, 0.0),
            no_rotation=True,
            parent_world_matrix=(
                (1.0000001192092896, 4.76837158203125e-7, 1.7484583736404602e-7, 0.0002016690996242687),
                (-1.7484549630353285e-7, -7.085781135174329e-7, 0.9999999403953552, -1.2602757215499878),
                (4.768372150465439e-7, -1.0000001192092896, -9.088388424061122e-7, 3.3709583282470703),
                (0.0, 0.0, 0.0, 1.0),
            ),
        )

        self.assertEqual(location, (0.0, 0.0, 0.0))

    def test_nonzero_rotation_no_rotation_attachment_keeps_offset(self) -> None:
        location = _scene_attachment_offset_to_blender(
            (0.9120000004768372, -1.2000000476837158, 1.0),
            (0.0, 0.0, 45.0),
            no_rotation=True,
            parent_world_matrix=(
                (1.0, 0.0, 0.0, 0.9120000004768372),
                (0.0, 1.0, 0.0, -1.2000000476837158),
                (0.0, 0.0, 1.0, 1.0),
                (0.0, 0.0, 0.0, 1.0),
            ),
        )

        self.assertEqual(location, (0.9120000004768372, -1.0, -1.2000000476837158))


if __name__ == "__main__":
    unittest.main()
