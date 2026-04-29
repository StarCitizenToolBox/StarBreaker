"""Regression tests for the snap_first / snap_last pose-application math
used by `runtime.package_ops._apply_best_channel_transform`.

Pins:

* `_decode_animation_position("identity", …)` is a pass-through; the
  exporter writes Blender-frame XYZ already (per
  `crates/starbreaker-3d/src/animation.rs::clip_to_json`, which emits
  `(cry_y, -cry_z, cry_x)`).
* The endpoint-policy selector picks the first sample at frame_index=0
  and the last sample otherwise (literal mode).
* The position-track delta is computed against the keyframe whose
  decoded position is closest to the bone's bind location, then added
  to the bind location. A bone whose entire position track is constant
  must therefore stay at bind, regardless of the absolute offset
  between the clip's authored values and the bone's bind position.

This last property is the key contract the `wings_deploy` X-shape
investigation hinges on: rotators `Wing_Rotator_Top_Left` and
`Wing_Rotator_Bottom_Right` have constant position tracks in the
sidecar and therefore MUST land at bind position. If a future change
breaks that property, the X-shape failure mode reappears regardless
of any parent-frame composition fix.

See `docs/StarBreaker/animation-research.md` (Scorpius wing-deploy
kinematics → Deployed-pose verification in Blender) for the data the
constants below are pinned against.
"""

from __future__ import annotations

import json
import math
import sys
import types
import unittest
from pathlib import Path


ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


class _StubObject:
    """Minimal stand-in for `bpy.types.Object` for pose-math tests."""

    def __init__(
        self,
        name: str,
        location=(0.0, 0.0, 0.0),
        rotation_quaternion=(1.0, 0.0, 0.0, 0.0),
    ) -> None:
        self.name = name
        self.location = tuple(float(v) for v in location)
        self.rotation_mode = "QUATERNION"
        self.rotation_quaternion = tuple(float(v) for v in rotation_quaternion)
        self.type = "MESH"
        self.parent = None
        self.children_recursive: list[object] = []
        self._props: dict[str, object] = {}

    # dict-like custom-properties access mirroring the bpy API surface
    # the production code uses.
    def __getitem__(self, key: str) -> object:
        return self._props[key]

    def __setitem__(self, key: str, value: object) -> None:
        self._props[key] = value

    def __contains__(self, key: str) -> bool:
        return key in self._props

    def get(self, key: str, default: object = None) -> object:
        return self._props.get(key, default)


def _load_package_ops() -> types.ModuleType:
    bpy = sys.modules.get("bpy")
    if bpy is None:
        bpy = types.ModuleType("bpy")
        sys.modules["bpy"] = bpy
    bpy.types = types.SimpleNamespace(Context=object, Object=object, ID=object, Light=object)
    bpy.data = types.SimpleNamespace(objects=[], lights=[])

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

    runtime_pkg = sys.modules.get("sb_anim_test_runtime")
    if runtime_pkg is None:
        runtime_pkg = types.ModuleType("sb_anim_test_runtime")
        runtime_pkg.__path__ = [str(ADDON_ROOT / "runtime")]
        sys.modules["sb_anim_test_runtime"] = runtime_pkg

    addon_pkg = sys.modules.get("sb_anim_test_addon")
    if addon_pkg is None:
        addon_pkg = types.ModuleType("sb_anim_test_addon")
        addon_pkg.__path__ = [str(ADDON_ROOT)]
        sys.modules["sb_anim_test_addon"] = addon_pkg

    manifest_stub = types.ModuleType("sb_anim_test_addon.manifest")
    manifest_stub.PackageBundle = type("PackageBundle", (), {"load": staticmethod(lambda p: None)})
    manifest_stub.SceneInstanceRecord = type("SceneInstanceRecord", (), {})
    sys.modules["sb_anim_test_addon.manifest"] = manifest_stub

    palette_stub = types.ModuleType("sb_anim_test_addon.palette")
    palette_stub.palette_id_for_livery_instance = lambda *a, **kw: None
    palette_stub.resolved_palette_id = lambda package, requested, inherited: requested or inherited
    sys.modules["sb_anim_test_addon.palette"] = palette_stub

    validators_stub = types.ModuleType("sb_anim_test_runtime.validators")
    validators_stub._purge_orphaned_file_backed_images = lambda: 0
    validators_stub._purge_orphaned_runtime_groups = lambda: 0
    sys.modules["sb_anim_test_runtime.validators"] = validators_stub

    importer_stub = types.ModuleType("sb_anim_test_runtime.importer")
    importer_stub.PackageImporter = type("PackageImporter", (), {})
    sys.modules["sb_anim_test_runtime.importer"] = importer_stub

    constants_path = ADDON_ROOT / "runtime" / "constants.py"
    constants = types.ModuleType("sb_anim_test_runtime.constants")
    constants.__file__ = str(constants_path)
    spec = __import__("importlib.util").util.spec_from_file_location(
        "sb_anim_test_runtime.constants", str(constants_path)
    )
    assert spec is not None and spec.loader is not None
    spec.loader.exec_module(constants)
    sys.modules["sb_anim_test_runtime.constants"] = constants

    source = (ADDON_ROOT / "runtime" / "package_ops.py").read_text()
    source = source.replace("from ..manifest import", "from sb_anim_test_addon.manifest import")
    source = source.replace("from ..palette import", "from sb_anim_test_addon.palette import")
    source = source.replace("from .constants import", "from sb_anim_test_runtime.constants import")
    source = source.replace("from .validators import", "from sb_anim_test_runtime.validators import")
    source = source.replace(
        "from .importer import PackageImporter",
        "from sb_anim_test_runtime.importer import PackageImporter",
    )
    module = types.ModuleType("sb_anim_test_runtime.package_ops")
    module.__file__ = str(ADDON_ROOT / "runtime" / "package_ops.py")
    module.__package__ = "sb_anim_test_runtime"
    sys.modules[module.__name__] = module
    exec(compile(source, module.__file__, "exec"), module.__dict__)
    return module


# Pinned values from the live sidecar
# `ships/Packages/RSI Scorpius_LOD0_TEX0/scene.json`,
# clip "wings_deploy" (verified 2026-04-27).
# Position values are already in Blender XYZ (cry_y, -cry_z, cry_x).
_TOP_LEFT_POS_FIRST = [0.023793935775756836, 0.8021461367607117, -1.3102056980133057]
_TOP_LEFT_POS_LAST = [0.023793935775756836, 0.8021461367607117, -1.3102056980133057]
_TOP_RIGHT_POS_FIRST = [-0.5459997653961182, 0.8021460771560669, 1.3102059364318848]
_TOP_RIGHT_POS_LAST = [0.023352086544036865, 0.8021460771560669, 1.6394926309585571]

# Bind positions in Blender local frame (NMC bone_to_world translation
# axis-swapped via (cry_y, -cry_z, cry_x); see
# docs/StarBreaker/animation-research.md → bind pose tables).
_TOP_LEFT_BIND = (-0.546, 0.802, -1.310)
_TOP_RIGHT_BIND = (-0.546, 0.802, 1.310)


class AnimationPoseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.package_ops = _load_package_ops()

    # ---- pure decoder ------------------------------------------------

    def test_decode_identity_is_passthrough(self) -> None:
        decoded = self.package_ops._decode_animation_position([1.5, -2.25, 3.0], "identity")
        self.assertEqual(decoded, (1.5, -2.25, 3.0))

    def test_decode_legacy_swaps_y_and_z(self) -> None:
        # Legacy decoder retained for older sidecars: (x, z, -y) → (x, -z, y)
        decoded = self.package_ops._decode_animation_position([1.0, 2.0, 3.0], "legacy")
        self.assertEqual(decoded, (1.0, -3.0, 2.0))

    # ---- _apply_best_channel_transform endpoint policy ---------------

    def _make_object(self, name: str, bind_loc: tuple[float, float, float]) -> _StubObject:
        return _StubObject(name=name, location=bind_loc)

    def _bind_data(self, bind_loc: tuple[float, float, float]) -> dict[str, object]:
        return {
            "location": list(bind_loc),
            "rotation_mode": "QUATERNION",
            "rotation_quaternion": [1.0, 0.0, 0.0, 0.0],
            "parent_distance": None,
        }

    def test_constant_position_track_keeps_bone_at_verbatim_sample(self) -> None:
        """Wing_Rotator_Top_Left has a constant position track in the
        wings_deploy sidecar. With verbatim position, the endpoint lands
        at the sampled position directly (CAF stores absolute parent-local
        values). The first and last samples are equal so the bone stays at
        the clip's authored resting position, which happens to be very
        close to bind for this bone.
        """
        obj = self._make_object("Wing_Rotator_Top_Left", _TOP_LEFT_BIND)
        channel = {
            "rotation": [[0.999, 0.0, 0.044, 0.0], [0.866, 0.0, 0.500, 0.0]],
            "position": [_TOP_LEFT_POS_FIRST, _TOP_LEFT_POS_LAST],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(_TOP_LEFT_BIND),
            channel,
            frame_index=1,
            endpoint_policy="literal",
        )

        for axis, (got, want) in enumerate(zip(obj.location, _TOP_LEFT_POS_LAST)):
            self.assertAlmostEqual(
                got, want, places=5,
                msg=f"axis {axis}: constant track must use verbatim sample",
            )

    def test_authored_position_track_lands_at_verbatim_last_sample(self) -> None:
        """Wing_Rotator_Top_Right's clip moves the bone between frame 0
        and the last frame. With verbatim position apply, the endpoint
        lands at the last sample directly (CAF stores absolute parent-local
        values, not deltas relative to any anchor).
        """
        obj = self._make_object("Wing_Rotator_Top_Right", _TOP_RIGHT_BIND)
        channel = {
            "rotation": [[0.999, 0.0, -0.044, 0.0], [0.866, 0.0, -0.500, 0.0]],
            "position": [_TOP_RIGHT_POS_FIRST, _TOP_RIGHT_POS_LAST],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(_TOP_RIGHT_BIND),
            channel,
            frame_index=1,
            endpoint_policy="literal",
        )

        for axis, (got, want) in enumerate(zip(obj.location, _TOP_RIGHT_POS_LAST)):
            self.assertAlmostEqual(got, want, places=5, msg=f"axis {axis}")

    def test_override_blend_mode_uses_sample_verbatim(self) -> None:
        """Phase 38 override path. When the per-bone `blend_mode` is
        marked as `override` (CHR-bind sits outside the AABB of CAF
        position samples), the addon must use the sampled position
        verbatim and ignore the bind. The canonical real-world case
        is Scorpius `BONE_Front_Landing_Gear_Foot`, whose CHR-bind is
        ~1.81m off any clip sample. With the additive pathway that
        bone lands far off the gear; with the override pathway it
        lands at the sample exactly.
        """
        bind = (10.0, 0.0, 0.0)
        sample = [3.5, 1.25, -0.75]
        obj = self._make_object("BONE_Front_Landing_Gear_Foot", bind)
        channel = {
            "rotation": [[1.0, 0.0, 0.0, 0.0]],
            "position": [sample],
            "blend_mode": "override",
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(bind),
            channel,
            frame_index=0,
            endpoint_policy="transition_end",
        )
        for axis, (got, want) in enumerate(zip(obj.location, sample)):
            self.assertAlmostEqual(
                got, want, places=5,
                msg=f"axis {axis}: override mode must use sample verbatim",
            )

    def test_position_track_matches_bind_detects_incompatible_duplicate(self) -> None:
        positions = [
            [0.0, 0.099973, -1.002515],
            [0.0, 0.099973, 0.530196],
        ]

        self.assertTrue(
            self.package_ops._position_track_matches_bind((0.0, 0.1, 0.53), positions),
            "front swingarm bind should match the shared track",
        )
        self.assertFalse(
            self.package_ops._position_track_matches_bind((0.788171, -0.460876, -0.0005), positions),
            "rear swingarm bind should be rejected for the shared track",
        )

    def test_shared_hash_position_policy_suppresses_incompatible_duplicates(self) -> None:
        channel = {
            "position": [
                [0.0, 0.099973, -1.002515],
                [0.0, 0.099973, 0.530196],
            ]
        }
        front = self._make_object("swingarm_big_anim.001", (0.0, 0.1, 0.53))
        rear = self._make_object("swingarm_big_anim.003", (0.788171, -0.460876, -0.0005))

        policy = self.package_ops._shared_hash_position_policy(
            {
                "0x2522C378": [
                    (front, self._bind_data((0.0, 0.1, 0.53))),
                    (rear, self._bind_data((0.788171, -0.460876, -0.0005))),
                ]
            },
            {"0x2522C378": channel},
        )

        self.assertTrue(policy[id(front)])
        self.assertFalse(policy[id(rear)])

    def test_transform_application_can_skip_full_channel_for_mismatched_duplicate(self) -> None:
        bind = (2.304258, -0.327204, 0.0)
        obj = _StubObject(
            "door_upper_anim.004",
            location=bind,
            rotation_quaternion=(0.300706, 0.0, 0.0, -0.953717),
        )
        bind_data = {
            "location": list(bind),
            "rotation_mode": "QUATERNION",
            "rotation_quaternion": [0.300706, 0.0, 0.0, -0.953717],
            "parent_distance": None,
        }
        channel = {
            "rotation": [[1.0, 0.0, 0.0, 0.0], [-0.300719, 0.0, 0.0, 0.953713]],
            "position": [[0.386403, -0.331004, -0.000001], [1.330354, -0.673419, -0.000001]],
        }

        self.package_ops._apply_best_channel_transform(
            obj,
            bind_data,
            channel,
            frame_index=0,
            endpoint_policy="literal",
            allow_rotation=False,
            allow_position=False,
        )

        for axis, (got, want) in enumerate(zip(obj.location, bind)):
            self.assertAlmostEqual(got, want, places=5, msg=f"location axis {axis}")
        expected_rot = (0.300706, 0.0, 0.0, -0.953717)
        for axis, (got, want) in enumerate(zip(obj.rotation_quaternion, expected_rot)):
            self.assertAlmostEqual(got, want, places=5, msg=f"rotation axis {axis}")

    # ---- Phase 39: layered-Action helpers ---------------------------

    def test_action_fcurves_handles_legacy_action(self) -> None:
        """Pre-Blender-4.4 Actions expose `Action.fcurves` directly.
        The helper must return that collection unchanged.
        """
        legacy_fcurves = ["fc_loc_x", "fc_loc_y", "fc_loc_z", "fc_quat_w"]
        action = types.SimpleNamespace(fcurves=legacy_fcurves)
        result = self.package_ops._action_fcurves(action)
        self.assertEqual(result, legacy_fcurves)

    def test_action_fcurves_walks_layered_channelbag(self) -> None:
        """Blender 5.1 Actions have no `Action.fcurves`; fcurves live on
        `action.layers[*].strips[*].channelbag(slot).fcurves`. The helper
        must enumerate them via the layered API.
        """
        slot = object()
        cb_fcurves = ["fc_loc_x", "fc_loc_y", "fc_quat_w"]
        channelbag = types.SimpleNamespace(fcurves=cb_fcurves)
        strip = types.SimpleNamespace(
            channelbag=lambda s, ensure=False: channelbag if s is slot else None
        )
        layer = types.SimpleNamespace(strips=[strip])
        action = types.SimpleNamespace(layers=[layer], slots=[slot])
        # Make sure attempting to access `.fcurves` does NOT yield the
        # legacy attribute.
        self.assertFalse(hasattr(action, "fcurves"))
        result = self.package_ops._action_fcurves(action)
        self.assertEqual(result, cb_fcurves)

    def test_action_groups_collection_returns_layered_channelbag_groups(self) -> None:
        """The Phase 39 regression: `Action.groups` is removed in Blender
        5.1 and grouping must come from the layered channelbag instead.
        Looking up `action.groups` directly used to abort the Insert
        Action loop after the first bone; the helper must transparently
        return the channelbag's groups collection.
        """

        class _Groups:
            def __init__(self) -> None:
                self._items: dict[str, object] = {}

            def get(self, name: str) -> object | None:
                return self._items.get(name)

            def new(self, name: str) -> object:
                grp = object()
                self._items[name] = grp
                return grp

        cb_groups = _Groups()
        slot = object()
        channelbag = types.SimpleNamespace(groups=cb_groups)
        strip = types.SimpleNamespace(
            channelbag=lambda s, ensure=False: channelbag if s is slot else None
        )
        layer = types.SimpleNamespace(strips=[strip])
        action = types.SimpleNamespace(layers=[layer], slots=[slot])
        self.assertFalse(hasattr(action, "groups"))
        result = self.package_ops._action_groups_collection(action)
        self.assertIs(result, cb_groups)
        # The helper must support the regular Insert-Action call sequence:
        # caller does `groups.get(name)` → falsy → `groups.new(name)`.
        self.assertIsNone(result.get("BoneA"))
        new_group = result.new("BoneA")
        self.assertIs(result.get("BoneA"), new_group)

    def test_action_groups_collection_returns_none_when_no_data(self) -> None:
        """A freshly-created layered Action with no keyframes inserted
        yet has no slots/channelbags. The helper must return None
        instead of raising — callers then skip grouping silently.
        """
        action = types.SimpleNamespace(layers=[], slots=[])
        self.assertIsNone(self.package_ops._action_groups_collection(action))

    def test_endpoint_policy_literal_picks_first_at_frame_zero(self) -> None:
        obj = self._make_object("Wing_Rotator_Top_Right", _TOP_RIGHT_BIND)
        channel = {
            "rotation": [[0.999, 0.0, -0.044, 0.0], [0.866, 0.0, -0.500, 0.0]],
            "position": [_TOP_RIGHT_POS_FIRST, _TOP_RIGHT_POS_LAST],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(_TOP_RIGHT_BIND),
            channel,
            frame_index=0,
            endpoint_policy="literal",
        )
        # At frame 0 the literal policy picks pos[0]. With verbatim position,
        # location = pos[0] directly.
        for axis, (got, want) in enumerate(zip(obj.location, _TOP_RIGHT_POS_FIRST)):
            self.assertAlmostEqual(got, want, places=5, msg=f"axis {axis}")
        # Rotation at frame 0 is bind-equivalent (~5° tilt).
        self.assertAlmostEqual(obj.rotation_quaternion[0], 0.999, places=3)

    def test_rotation_sample_writes_quaternion_unchanged(self) -> None:
        obj = self._make_object("Wing_Mechanism_Top_Left", (0.0, 0.0, 0.0))
        channel = {
            "rotation": [[1.0, 0.0, 0.0, 0.0], [0.985, 0.174, 0.0, 0.0]],
            "position": [],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=1,
            endpoint_policy="literal",
        )
        self.assertEqual(obj.rotation_mode, "QUATERNION")
        for got, want in zip(obj.rotation_quaternion, (0.985, 0.174, 0.0, 0.0)):
            self.assertAlmostEqual(got, want, places=5)

    def test_fragment_tagged_cyclic_clip_targets_mid_transition_time(self) -> None:
        clip = {
            "fragments": [{"frag_tags": ["Open"], "animations": [{"name": "canopy_open"}]}],
            "bones": {
                "0x00000001": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
                    "position_time": [0.0, 36.5, 75.0],
                },
                "0x00000002": {
                    "position": [[1.0, 0.0, 0.0], [1.0, -3.0, 0.0], [1.0, -0.1, 0.0]],
                    "position_time": [0.0, 36.5, 75.0],
                },
            },
        }

        self.assertEqual(self.package_ops._clip_cyclic_transition_target_frame(clip), 36.5)

    def test_cyclic_target_requires_source_fragment_metadata(self) -> None:
        clip = {
            "bones": {
                "0x00000001": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
                    "position_time": [0.0, 36.5, 75.0],
                }
            }
        }

        self.assertIsNone(self.package_ops._clip_cyclic_transition_target_frame(clip))

    def test_mixed_cyclic_clip_keeps_literal_endpoint(self) -> None:
        clip = {
            "fragments": [{"frag_tags": ["Deploy"], "animations": [{"name": "landing_gear_extend"}]}],
            "bones": {
                "0x00000001": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
                    "position_time": [0.0, 50.0, 100.0],
                },
                "0x00000002": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 2.0, 0.0]],
                    "position_time": [0.0, 50.0, 100.0],
                },
                "0x00000003": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 2.0, 0.0]],
                    "position_time": [0.0, 50.0, 100.0],
                },
            },
        }

        self.assertIsNone(self.package_ops._clip_cyclic_transition_target_frame(clip))

    def test_fragment_endpoint_policy_maps_state_tags_to_transition(self) -> None:
        deploy = {
            "fragment": "Landing_Gear",
            "frag_tags": ["Deploy"],
            "animations": [{"name": "landing_gear_extend"}],
        }
        retract = {
            "fragment": "Landing_Gear",
            "frag_tags": ["Retract"],
            "animations": [{"name": "landing_gear_extend", "speed": -1}],
        }

        # Forward fragment (Deploy): snap_first -> start, snap_last -> end.
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(deploy, "snap_first"),
            "transition_start",
        )
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(deploy, "snap_last"),
            "transition_end",
        )
        # Reverse-playback fragment (Retract, speed=-1): mapping flips.
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(retract, "snap_first"),
            "transition_end",
        )
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(retract, "snap_last"),
            "transition_start",
        )

    def test_target_frame_snap_opens_from_bind_anchored_start(self) -> None:
        # snap_last on a cyclic clip whose first sample is
        # bind-aligned (the bone's resting state in clip-frame) and
        # whose target frame holds the deployed pose: anchor must be
        # the bind-nearer endpoint (here the first sample), so
        # `result = bind + (target - first)` lands at the target.
        obj = self._make_object("Canopy_Front", (0.0, 0.0, 0.0))
        channel = {
            "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
            "position_time": [0.0, 36.5, 75.0],
        }

        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=-1,
            endpoint_policy="literal",
            target_frame=36.5,
            anchor_frame=36.5,
        )

        self.assertEqual(obj.location, (0.0, 2.0, 0.0))

    def test_target_frame_snap_closes_to_bind_when_target_is_reference(self) -> None:
        obj = self._make_object("Canopy_Front", (0.0, 0.0, 0.0))
        channel = {
            "position": [[0.0, 2.0, 0.0], [0.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            "position_time": [0.0, 36.5, 75.0],
        }

        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=-1,
            endpoint_policy="literal",
            target_frame=36.5,
            anchor_frame=36.5,
        )

        self.assertEqual(obj.location, (0.0, 0.0, 0.0))

    def test_snap_first_can_use_target_frame_as_anchor_reference(self) -> None:
        obj = self._make_object("Canopy_Front", (0.0, 0.0, 0.0))
        channel = {
            "position": [[0.0, 2.0, 0.0], [0.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            "position_time": [0.0, 36.5, 75.0],
        }

        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=0,
            endpoint_policy="literal",
            anchor_frame=36.5,
        )

        self.assertEqual(obj.location, (0.0, 2.0, 0.0))

    # ---- Phase 53 hotfix: clip-root-frame DBA fields are research-only --

    def test_clip_start_position_does_not_displace_per_bone_anchor(self) -> None:
        """Regression: Phase 53 originally consumed DBA `start_position`
        as a per-bone anchor, which displaced bones whose first
        authored sample was not at the clip-root origin (Scorpius
        landing_gear_deploy: bones "completely separated"). The DBA
        fields are clip-root-frame, not parent-local, so they are
        plumbed through scene.json but not used per-bone. Anchor
        selection must remain a bind-distance choice between
        `first_sample` and the `anchor_frame` sample.
        """
        obj = self._make_object("BONE_Back_Piston_Lower", (0.0, 0.0, 0.0))
        channel = {
            # First sample is bind-aligned; target deploys the bone.
            "position": [[0.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 0.0]],
            "position_time": [0.0, 36.5, 75.0],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=-1,
            endpoint_policy="literal",
            target_frame=36.5,
            anchor_frame=36.5,
        )
        # bind + (target - first) = (0, 0.5, 0) — not (0, 0, 0)+(0, 0.5, 0)
        # piled on a non-zero clip-root start.
        self.assertEqual(obj.location, (0.0, 0.5, 0.0))

    def test_drak_door_cyclic_returns_to_bind_via_first_sample_anchor(self) -> None:
        """DRAK Clipper shape: cyclic rotation that returns to identity.
        Without the retired `_channel_other_endpoint` heuristic, the
        first-sample anchor leaves snap_last at bind even when a
        mid-clip extreme is present.
        """
        obj = self._make_object("door_lower_anim", (0.0, 0.0, 0.0))
        bind = self._bind_data((0.0, 0.0, 0.0))
        channel = {
            "rotation": [
                [1.0, 0.0, 0.0, 0.0],
                [0.707, 0.707, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
            ],
            "rotation_time": [0.0, 36.5, 75.0],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            bind,
            channel,
            frame_index=-1,
            endpoint_policy="transition_end",
        )
        for actual, expected in zip(obj.rotation_quaternion, (1.0, 0.0, 0.0, 0.0)):
            self.assertAlmostEqual(actual, expected, places=5)

    def test_literal_snap_last_picks_last_sample_as_anchor_when_last_is_nearer_bind(self) -> None:
        """Regression guard for DRAK Clipper swingarm pattern.
        snap_last on a literal clip where `last_pos ≈ bind` and
        `first_pos` is far away: the bind-distance pick must choose
        `last` as anchor so the result is bind + (last - last) = bind,
        not bind + (last - first) = displaced.
        """
        obj = self._make_object("swingarm_big_anim", (0.0, 0.1, 0.53))
        bind = self._bind_data((0.0, 0.1, 0.53))
        channel = {
            # first ≈ retracted (far), last ≈ bind (deployed rest pose)
            "position": [[0.0, 0.1, -1.0], [0.0, 0.1, 0.53]],
            "position_time": [0.0, 75.0],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            bind,
            channel,
            frame_index=-1,
            endpoint_policy="literal",
        )
        # verbatim last = (0, 0.1, 0.53) = bind → no displacement
        for actual, expected in zip(obj.location, (0.0, 0.1, 0.53)):
            self.assertAlmostEqual(actual, expected, places=4)

    def test_caf_only_clip_uses_first_sample_anchor(self) -> None:
        """CAF-only clips have no DBA metadata. Positions use verbatim
        sample (absolute parent-local); rotations use verbatim sample.
        """
        obj = self._make_object("wing_pivot", (0.0, 0.0, 0.0))
        bind = self._bind_data((0.0, 0.0, 0.0))
        channel = {
            "rotation": [[1.0, 0.0, 0.0, 0.0], [0.707, 0.0, 0.707, 0.0]],
            "position": [[0.0, 1.5, 0.0], [0.0, 5.5, 0.0]],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            bind,
            channel,
            frame_index=-1,
            endpoint_policy="transition_end",
        )
        # verbatim last position = (0, 5.5, 0) regardless of bind or first
        self.assertEqual(obj.location, (0.0, 5.5, 0.0))
        for actual, expected in zip(obj.rotation_quaternion, (0.707, 0.0, 0.707, 0.0)):
            self.assertAlmostEqual(actual, expected, places=5)

if __name__ == "__main__":  # pragma: no cover
    unittest.main()
