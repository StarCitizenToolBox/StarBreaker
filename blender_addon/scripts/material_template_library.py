from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import sys
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import bpy

from node_layout import auto_layout_node_tree


BLENDER_ADDON_DIR = SCRIPT_DIR.parent
RESOURCES_DIR = BLENDER_ADDON_DIR / "starbreaker_addon" / "resources"
CONTRACT_PATH = RESOURCES_DIR / "material_template_contract.json"
LIBRARY_PATH = RESOURCES_DIR / "material_templates.blend"

GROUP_METADATA_KEY = "sb_group_metadata_json"
INPUT_METADATA_KEY = "sb_input_metadata_json"
SHADER_FAMILIES_KEY = "sb_shader_families_json"
VERSION_KEY = "sb_group_version"
SHADER_OUTPUT_KEY = "sb_shader_output"


@dataclass(frozen=True)
class ValidationFailure:
    group_name: str
    message: str


def _json_default(value: Any) -> Any:
    if isinstance(value, Path):
        return value.as_posix()
    raise TypeError(f"Unsupported JSON value: {type(value)!r}")


def resource_contract_path() -> Path:
    return CONTRACT_PATH


def resource_library_path() -> Path:
    return LIBRARY_PATH


def load_contract(path: Path | None = None) -> dict[str, Any]:
    contract_path = path or resource_contract_path()
    with contract_path.open("r", encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise ValueError(f"Expected contract JSON object in {contract_path}")
    return payload


def _serialize_default_value(value: Any) -> Any:
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if hasattr(value, "to_list"):
        return value.to_list()
    if isinstance(value, (list, tuple)):
        return list(value)
    return None


def _metadata_json(value: Any) -> dict[str, Any]:
    if not value:
        return {}
    if isinstance(value, str):
        return dict(json.loads(value))
    if isinstance(value, dict):
        return dict(value)
    return {}


def _interface_sockets(node_tree: bpy.types.NodeTree, in_out: str) -> list[Any]:
    sockets: list[Any] = []
    for item in node_tree.interface.items_tree:
        if getattr(item, "item_type", None) != "SOCKET":
            continue
        if getattr(item, "in_out", None) != in_out:
            continue
        sockets.append(item)
    return sockets


def _interface_socket_names(node_tree: bpy.types.NodeTree, in_out: str) -> list[str]:
    return [socket.name for socket in _interface_sockets(node_tree, in_out)]


def _input_metadata(group_data: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        item["name"]: {
            "socket_type": item.get("socket_type", "NodeSocketColor"),
            "semantic": item.get("semantic"),
            "source_slot": item.get("source_slot"),
            "required": bool(item.get("required", False)),
            "default_value": item.get("default_value"),
        }
        for item in group_data.get("inputs", [])
    }


def _set_interface_defaults(interface_socket: Any, metadata: dict[str, Any]) -> None:
    default_value = metadata.get("default_value")
    if default_value is None or not hasattr(interface_socket, "default_value"):
        return
    try:
        interface_socket.default_value = default_value
    except Exception:
        return


def create_group_from_contract(group_data: dict[str, Any]) -> bpy.types.ShaderNodeTree:
    group_name = str(group_data["name"])
    if group_name in bpy.data.node_groups:
        raise ValueError(f"Node group {group_name} already exists in the current file")

    node_tree = bpy.data.node_groups.new(group_name, "ShaderNodeTree")
    node_tree.use_fake_user = True
    input_metadata = _input_metadata(group_data)

    for socket_data in group_data.get("inputs", []):
        metadata = input_metadata[socket_data["name"]]
        interface_socket = node_tree.interface.new_socket(
            name=socket_data["name"],
            in_out="INPUT",
            socket_type=metadata["socket_type"],
        )
        _set_interface_defaults(interface_socket, metadata)

    shader_output_name = str(group_data.get("shader_output", "Shader"))
    node_tree.interface.new_socket(name=shader_output_name, in_out="OUTPUT", socket_type="NodeSocketShader")

    nodes = node_tree.nodes
    links = node_tree.links
    group_input = nodes.new("NodeGroupInput")
    group_input.location = (-300.0, 0.0)
    placeholder = nodes.new("ShaderNodeBsdfTransparent")
    placeholder.location = (0.0, 0.0)
    group_output = nodes.new("NodeGroupOutput")
    group_output.location = (260.0, 0.0)
    links.new(placeholder.outputs[0], group_output.inputs[shader_output_name])

    node_tree[GROUP_METADATA_KEY] = json.dumps(group_data.get("metadata", {}), default=_json_default, sort_keys=True)
    node_tree[INPUT_METADATA_KEY] = json.dumps(input_metadata, default=_json_default, sort_keys=True)
    node_tree[SHADER_FAMILIES_KEY] = json.dumps(group_data.get("shader_families", []), sort_keys=True)
    node_tree[VERSION_KEY] = int(group_data.get("version", 1))
    node_tree[SHADER_OUTPUT_KEY] = shader_output_name
    return node_tree


def build_library(contract_path: Path | None = None, output_path: Path | None = None) -> dict[str, Any]:
    contract = load_contract(contract_path)
    groups_data = list(contract.get("groups", []))
    existing = [str(group_data["name"]) for group_data in groups_data if str(group_data["name"]) in bpy.data.node_groups]
    if existing:
        raise ValueError(f"Refusing to overwrite existing node groups in the current file: {', '.join(existing)}")

    created_groups = [create_group_from_contract(group_data) for group_data in groups_data]
    library_path = output_path or resource_library_path()
    library_path.parent.mkdir(parents=True, exist_ok=True)
    bpy.data.libraries.write(str(library_path), set(created_groups), fake_user=True)

    for group in created_groups:
        bpy.data.node_groups.remove(group)

    return {
        "library_path": library_path.as_posix(),
        "group_count": len(groups_data),
        "group_names": [str(group_data["name"]) for group_data in groups_data],
    }


def gather_group_contract(node_tree: bpy.types.NodeTree) -> dict[str, Any]:
    group_metadata = _metadata_json(node_tree.get(GROUP_METADATA_KEY))
    input_metadata = _metadata_json(node_tree.get(INPUT_METADATA_KEY))
    shader_families = json.loads(node_tree.get(SHADER_FAMILIES_KEY, "[]"))
    shader_output = str(node_tree.get(SHADER_OUTPUT_KEY, "Shader"))

    inputs = []
    for socket in _interface_sockets(node_tree, "INPUT"):
        metadata = dict(input_metadata.get(socket.name, {}))
        inputs.append(
            {
                "name": socket.name,
                "socket_type": metadata.get("socket_type") or getattr(socket, "bl_socket_idname", "NodeSocketColor"),
                "semantic": metadata.get("semantic"),
                "source_slot": metadata.get("source_slot"),
                "required": bool(metadata.get("required", False)),
                "default_value": metadata.get("default_value", _serialize_default_value(getattr(socket, "default_value", None))),
            }
        )

    return {
        "name": node_tree.name,
        "shader_families": list(shader_families),
        "version": int(node_tree.get(VERSION_KEY, 1)),
        "shader_output": shader_output,
        "inputs": _ordered_contract_inputs(inputs),
        "metadata": group_metadata,
    }


def export_contract(output_path: Path | None = None) -> dict[str, Any]:
    groups = [group for group in bpy.data.node_groups if group.name.startswith("SB_")]
    groups.sort(key=lambda group: group.name)
    payload = {
        "schema_version": 1,
        "generated_from": resource_library_path().name,
        "groups": [gather_group_contract(group) for group in groups],
        "metadata": {
            "status": "blend_export",
            "group_count": len(groups),
        },
    }
    target_path = output_path or resource_contract_path()
    target_path.parent.mkdir(parents=True, exist_ok=True)
    target_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return payload


def validate_library(expected_contract_path: Path | None = None) -> list[ValidationFailure]:
    expected = load_contract(expected_contract_path)
    expected_groups = {group["name"]: group for group in expected.get("groups", [])}
    live_groups = {group.name: group for group in bpy.data.node_groups if group.name.startswith("SB_")}
    failures: list[ValidationFailure] = []

    for group_name, expected_group in expected_groups.items():
        live_group = live_groups.get(group_name)
        if live_group is None:
            failures.append(ValidationFailure(group_name, "missing group"))
            continue

        live_contract = gather_group_contract(live_group)
        expected_inputs = [(item["name"], item.get("socket_type", "NodeSocketColor")) for item in expected_group.get("inputs", [])]
        live_inputs = [(item["name"], item.get("socket_type", "NodeSocketColor")) for item in live_contract.get("inputs", [])]
        if expected_inputs != live_inputs:
            failures.append(
                ValidationFailure(
                    group_name,
                    f"input mismatch: expected {expected_inputs}, found {live_inputs}",
                )
            )

    for group_name in sorted(set(live_groups) - set(expected_groups)):
        failures.append(ValidationFailure(group_name, "unexpected extra group"))
    return failures


def _node_socket(collection: Any, *names: str) -> Any | None:
    for name in names:
        socket = collection.get(name)
        if socket is not None:
            return socket
    return None


def _group_input_socket(group_input: bpy.types.Node, name: str) -> Any | None:
    return group_input.outputs.get(name)


def _group_input_socket_by_semantic(group_input: bpy.types.Node, node_tree: bpy.types.NodeTree, *semantics: str) -> Any | None:
    wanted = set(semantics)
    input_metadata = _metadata_json(node_tree.get(INPUT_METADATA_KEY))
    for socket_name in _interface_socket_names(node_tree, "INPUT"):
        metadata = input_metadata.get(socket_name, {})
        if metadata.get("semantic") not in wanted:
            continue
        socket = _group_input_socket(group_input, socket_name)
        if socket is not None:
            return socket
    return None


def _first_group_input_socket(group_input: bpy.types.Node, node_tree: bpy.types.NodeTree) -> Any | None:
    for socket_name in _interface_socket_names(node_tree, "INPUT"):
        socket = _group_input_socket(group_input, socket_name)
        if socket is not None:
            return socket
    return None


def _first_group_color_socket(group_input: bpy.types.Node, node_tree: bpy.types.NodeTree) -> Any | None:
    input_metadata = _metadata_json(node_tree.get(INPUT_METADATA_KEY))
    for socket_name in _interface_socket_names(node_tree, "INPUT"):
        metadata = input_metadata.get(socket_name, {})
        socket_type = metadata.get("socket_type")
        if socket_type != "NodeSocketColor":
            continue
        socket = _group_input_socket(group_input, socket_name)
        if socket is not None:
            return socket
    return None


def _ordered_contract_inputs(inputs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    indexed = {item["name"]: item for item in inputs}
    emitted: set[str] = set()
    ordered: list[dict[str, Any]] = []

    def emit(name: str) -> None:
        if name in emitted:
            return
        item = indexed.get(name)
        if item is None:
            return
        ordered.append(item)
        emitted.add(name)

    for item in inputs:
        name = item["name"]
        if name.endswith("_alpha") and name.removesuffix("_alpha") in indexed:
            continue
        emit(name)
        emit(f"{name}_alpha")

    for item in inputs:
        emit(item["name"])
    return ordered


def _connect_standard_normal(
    node_tree: bpy.types.NodeTree,
    group_input: bpy.types.Node,
    normal_socket: Any,
    *semantics: str,
) -> None:
    source_socket = _group_input_socket_by_semantic(group_input, node_tree, *semantics)
    if source_socket is None:
        return
    _connect_normal_map(node_tree, group_input, source_socket.name, normal_socket)


def _connect_surface_mix(
    node_tree: bpy.types.NodeTree,
    surface_socket: Any,
    output_socket: Any,
    *,
    factor_socket: Any | None,
    factor_default: float,
    mix_location: tuple[float, float],
    transparent_location: tuple[float, float],
) -> None:
    transparent = node_tree.nodes.new("ShaderNodeBsdfTransparent")
    transparent.location = transparent_location
    mix = node_tree.nodes.new("ShaderNodeMixShader")
    mix.location = mix_location
    mix.inputs[0].default_value = factor_default
    if factor_socket is not None:
        node_tree.links.new(factor_socket, mix.inputs[0])
    node_tree.links.new(transparent.outputs[0], mix.inputs[1])
    node_tree.links.new(surface_socket, mix.inputs[2])
    node_tree.links.new(mix.outputs[0], output_socket)


def _ensure_group_from_contract(group_name: str) -> bpy.types.NodeTree | None:
    node_tree = bpy.data.node_groups.get(group_name)
    if node_tree is not None:
        node_tree.use_fake_user = True
        return node_tree

    group_data = next((group for group in load_contract().get("groups", []) if group.get("name") == group_name), None)
    if group_data is None:
        return None
    node_tree = create_group_from_contract(group_data)
    node_tree.use_fake_user = True
    return node_tree


def _palette_input_names(node_tree: bpy.types.NodeTree) -> list[str]:
    group_metadata = _metadata_json(node_tree.get(GROUP_METADATA_KEY))
    return [str(name) for name in group_metadata.get("proposed_palette_inputs", []) if str(name).startswith("Palette_")]


def _palette_input_semantic(socket_name: str) -> str:
    return f"palette_{socket_name.removeprefix('Palette_').lower()}"


def _ensure_palette_inputs(node_tree: bpy.types.NodeTree) -> None:
    palette_inputs = _palette_input_names(node_tree)
    if not palette_inputs:
        return
    existing_inputs = {socket.name for socket in _interface_sockets(node_tree, "INPUT")}
    input_metadata = _metadata_json(node_tree.get(INPUT_METADATA_KEY))
    changed = False
    for socket_name in palette_inputs:
        if socket_name not in existing_inputs:
            interface_socket = node_tree.interface.new_socket(name=socket_name, in_out="INPUT", socket_type="NodeSocketColor")
            _set_interface_defaults(interface_socket, {"default_value": [1.0, 1.0, 1.0, 1.0]})
            changed = True
        if socket_name not in input_metadata:
            input_metadata[socket_name] = {
                "socket_type": "NodeSocketColor",
                "semantic": _palette_input_semantic(socket_name),
                "source_slot": None,
                "required": False,
                "default_value": [1.0, 1.0, 1.0, 1.0],
            }
            changed = True
    if changed:
        node_tree[INPUT_METADATA_KEY] = json.dumps(input_metadata, default=_json_default, sort_keys=True)


def _interface_socket(node_tree: bpy.types.NodeTree, socket_name: str, in_out: str) -> Any | None:
    for socket in _interface_sockets(node_tree, in_out):
        if socket.name == socket_name:
            return socket
    return None


def _ensure_input_socket(
    node_tree: bpy.types.NodeTree,
    socket_name: str,
    socket_type: str,
    *,
    after_socket_name: str | None = None,
    before_socket_name: str | None = None,
    semantic: str | None,
    source_slot: str | None,
    required: bool,
    default_value: Any,
) -> None:
    existing_input_sockets = _interface_sockets(node_tree, "INPUT")
    existing_inputs = {socket.name for socket in existing_input_sockets}
    input_metadata = _metadata_json(node_tree.get(INPUT_METADATA_KEY))
    changed = False
    interface_socket = _interface_socket(node_tree, socket_name, "INPUT")

    if interface_socket is not None and getattr(interface_socket, "bl_socket_idname", None) != socket_type:
        input_index = next(
            index for index, socket in enumerate(existing_input_sockets) if socket.name == socket_name
        )
        node_tree.interface.remove(interface_socket)
        interface_socket = node_tree.interface.new_socket(name=socket_name, in_out="INPUT", socket_type=socket_type)
        node_tree.interface.move(interface_socket, input_index)
        _set_interface_defaults(interface_socket, {"default_value": default_value})
        existing_inputs.add(socket_name)
        changed = True

    if socket_name not in existing_inputs:
        interface_socket = node_tree.interface.new_socket(name=socket_name, in_out="INPUT", socket_type=socket_type)
        _set_interface_defaults(interface_socket, {"default_value": default_value})
        changed = True
    elif interface_socket is not None:
        _set_interface_defaults(interface_socket, {"default_value": default_value})

    if interface_socket is not None and before_socket_name is not None:
        ordered_inputs = _interface_sockets(node_tree, "INPUT")
        current_index = next((index for index, socket in enumerate(ordered_inputs) if socket.name == socket_name), None)
        before_index = next((index for index, socket in enumerate(ordered_inputs) if socket.name == before_socket_name), None)
        if current_index is not None and before_index is not None and current_index != before_index:
            node_tree.interface.move(interface_socket, before_index)
            changed = True

    if interface_socket is not None and after_socket_name is not None:
        ordered_inputs = _interface_sockets(node_tree, "INPUT")
        current_index = next((index for index, socket in enumerate(ordered_inputs) if socket.name == socket_name), None)
        after_index = next((index for index, socket in enumerate(ordered_inputs) if socket.name == after_socket_name), None)
        if current_index is not None and after_index is not None and current_index != after_index + 1:
            node_tree.interface.move(interface_socket, after_index + 1)
            changed = True

    metadata = input_metadata.get(socket_name)
    desired_metadata = {
        "socket_type": socket_type,
        "semantic": semantic,
        "source_slot": source_slot,
        "required": required,
        "default_value": default_value,
    }
    if metadata != desired_metadata:
        input_metadata[socket_name] = desired_metadata
        changed = True

    if changed:
        node_tree[INPUT_METADATA_KEY] = json.dumps(input_metadata, default=_json_default, sort_keys=True)


def _remove_input_socket(node_tree: bpy.types.NodeTree, socket_name: str) -> None:
    interface_socket = _interface_socket(node_tree, socket_name, "INPUT")
    input_metadata = _metadata_json(node_tree.get(INPUT_METADATA_KEY))
    changed = False
    if interface_socket is not None:
        node_tree.interface.remove(interface_socket)
        changed = True
    if socket_name in input_metadata:
        input_metadata.pop(socket_name, None)
        changed = True
    if changed:
        node_tree[INPUT_METADATA_KEY] = json.dumps(input_metadata, default=_json_default, sort_keys=True)


def _ensure_socket_precedes(node_tree: bpy.types.NodeTree, first_socket_name: str, second_socket_name: str) -> None:
    ordered_inputs = _interface_sockets(node_tree, "INPUT")
    first_socket = next((socket for socket in ordered_inputs if socket.name == first_socket_name), None)
    second_socket = next((socket for socket in ordered_inputs if socket.name == second_socket_name), None)
    if first_socket is None or second_socket is None:
        return
    first_index = next((index for index, socket in enumerate(ordered_inputs) if socket.name == first_socket_name), None)
    second_index = next((index for index, socket in enumerate(ordered_inputs) if socket.name == second_socket_name), None)
    if first_index is None or second_index is None:
        return
    if first_index > second_index:
        node_tree.interface.move(first_socket, second_index)
        return
    if second_index != first_index + 1:
        node_tree.interface.move(second_socket, first_index + 1)


def _reorder_input_sockets(node_tree: bpy.types.NodeTree, ordered_socket_names: list[str]) -> None:
    current_inputs = _interface_sockets(node_tree, "INPUT")
    input_metadata = _metadata_json(node_tree.get(INPUT_METADATA_KEY))
    socket_specs: dict[str, dict[str, Any]] = {}

    for socket in current_inputs:
        metadata = dict(input_metadata.get(socket.name, {}))
        socket_specs[socket.name] = {
            "socket_type": metadata.get("socket_type") or getattr(socket, "bl_socket_idname", "NodeSocketColor"),
            "default_value": metadata.get("default_value", _serialize_default_value(getattr(socket, "default_value", None))),
        }

    desired_names: list[str] = []
    for socket_name in ordered_socket_names:
        if socket_name in socket_specs and socket_name not in desired_names:
            desired_names.append(socket_name)
    for socket in current_inputs:
        if socket.name not in desired_names:
            desired_names.append(socket.name)

    for socket in list(current_inputs):
        node_tree.interface.remove(socket)

    for socket_name in desired_names:
        spec = socket_specs.get(socket_name)
        if spec is None:
            continue
        interface_socket = node_tree.interface.new_socket(
            name=socket_name,
            in_out="INPUT",
            socket_type=spec["socket_type"],
        )
        _set_interface_defaults(interface_socket, {"default_value": spec["default_value"]})


def _ensure_paired_alpha_input(
    node_tree: bpy.types.NodeTree,
    color_socket_name: str,
    *,
    semantic: str,
    source_slot: str,
) -> str:
    alpha_socket_name = f"{color_socket_name}_alpha"
    _ensure_input_socket(
        node_tree,
        alpha_socket_name,
        "NodeSocketFloat",
        semantic=f"{semantic}_alpha",
        source_slot=source_slot,
        required=False,
        default_value=0.0,
    )
    _ensure_socket_precedes(node_tree, color_socket_name, alpha_socket_name)
    return alpha_socket_name


def _combine_value_sockets(
    node_tree: bpy.types.NodeTree,
    sockets: list[Any | None],
    *,
    x: float,
    y: float,
) -> Any | None:
    active_sockets = [socket for socket in sockets if socket is not None]
    if not active_sockets:
        return None
    current_socket = active_sockets[0]
    for index, socket in enumerate(active_sockets[1:], start=1):
        maximum = node_tree.nodes.new("ShaderNodeMath")
        maximum.location = (x + 180.0 * index, y - 40.0 * index)
        maximum.operation = "MAXIMUM"
        node_tree.links.new(current_socket, maximum.inputs[0])
        node_tree.links.new(socket, maximum.inputs[1])
        current_socket = maximum.outputs[0]
    return current_socket


def _reset_group_nodes(node_tree: bpy.types.NodeTree) -> tuple[bpy.types.Node, bpy.types.Node]:
    node_tree.nodes.clear()
    group_input = node_tree.nodes.new("NodeGroupInput")
    group_input.location = (-640.0, 0.0)
    group_output = node_tree.nodes.new("NodeGroupOutput")
    group_output.location = (420.0, 0.0)
    return group_input, group_output


def _connect_normal_map(node_tree: bpy.types.NodeTree, group_input: bpy.types.Node, input_name: str, target_socket: Any) -> None:
    source_socket = _group_input_socket(group_input, input_name)
    if source_socket is None or target_socket is None:
        return
    normal_map = node_tree.nodes.new("ShaderNodeNormalMap")
    normal_map.location = (-220.0, -220.0)
    node_tree.links.new(source_socket, normal_map.inputs["Color"])
    node_tree.links.new(normal_map.outputs["Normal"], target_socket)


def _rgb_to_bw(node_tree: bpy.types.NodeTree, source_socket: Any, location: tuple[float, float]) -> Any:
    rgb_to_bw = node_tree.nodes.new("ShaderNodeRGBToBW")
    rgb_to_bw.location = location
    node_tree.links.new(source_socket, rgb_to_bw.inputs["Color"])
    return rgb_to_bw.outputs["Val"]


def _white_color_socket(node_tree: bpy.types.NodeTree, location: tuple[float, float]) -> Any:
    rgb = node_tree.nodes.new("ShaderNodeRGB")
    rgb.location = location
    rgb.outputs[0].default_value = (1.0, 1.0, 1.0, 1.0)
    return rgb.outputs[0]


def _cull_close_reflections(
    node_tree: bpy.types.NodeTree,
    surface_socket: Any,
    *,
    max_distance: float,
    location: tuple[float, float],
) -> Any:
    """Return a shader socket that replaces the decal/POM surface with a
    Transparent BSDF whenever a glossy ray samples it from within
    ``max_distance`` of another reflecting surface. Decals and POM
    surfaces sit slightly above the hull they annotate; close-range
    reflections (e.g. a shiny floor right underneath) double-sample the
    decal as a ghost offset from the hull. Distant reflections are
    unaffected because the angular offset is imperceptible at range.

    Gate: ``is_glossy_ray AND ray_length < max_distance``.
    """
    if surface_socket is None or max_distance <= 0.0:
        return surface_socket

    light_path = node_tree.nodes.new("ShaderNodeLightPath")
    light_path.location = (location[0] - 380.0, location[1])

    distance_gate = node_tree.nodes.new("ShaderNodeMath")
    distance_gate.operation = "LESS_THAN"
    distance_gate.label = "close reflection"
    distance_gate.use_clamp = True
    distance_gate.location = (location[0] - 200.0, location[1] - 40.0)
    ray_length = _node_socket(light_path.outputs, "Ray Length")
    if ray_length is not None:
        node_tree.links.new(ray_length, distance_gate.inputs[0])
    distance_gate.inputs[1].default_value = float(max_distance)

    combine = node_tree.nodes.new("ShaderNodeMath")
    combine.operation = "MULTIPLY"
    combine.use_clamp = True
    combine.label = "cull reflection"
    combine.location = (location[0] - 40.0, location[1] - 20.0)
    glossy = _node_socket(light_path.outputs, "Is Glossy Ray")
    if glossy is not None:
        node_tree.links.new(glossy, combine.inputs[0])
    node_tree.links.new(distance_gate.outputs[0], combine.inputs[1])

    transparent = node_tree.nodes.new("ShaderNodeBsdfTransparent")
    transparent.location = (location[0] - 40.0, location[1] - 180.0)

    mix = node_tree.nodes.new("ShaderNodeMixShader")
    mix.label = "reflection cull"
    mix.location = location
    node_tree.links.new(combine.outputs[0], mix.inputs[0])
    node_tree.links.new(surface_socket, mix.inputs[1])
    node_tree.links.new(transparent.outputs[0], mix.inputs[2])
    return mix.outputs[0]


def _connect_disable_shadow(
    node_tree: bpy.types.NodeTree,
    group_input: bpy.types.Node,
    surface_socket: Any,
    output_socket: Any,
    *,
    mix_location: tuple[float, float],
    transparent_location: tuple[float, float],
    light_path_location: tuple[float, float],
    math_location: tuple[float, float],
) -> None:
    disable_shadow_socket = _group_input_socket(group_input, "Disable Shadow")
    if disable_shadow_socket is None or surface_socket is None or output_socket is None:
        node_tree.links.new(surface_socket, output_socket)
        return

    light_path = node_tree.nodes.new("ShaderNodeLightPath")
    light_path.location = light_path_location
    shadow_mix_factor = node_tree.nodes.new("ShaderNodeMath")
    shadow_mix_factor.location = math_location
    shadow_mix_factor.operation = "MULTIPLY"
    shadow_mix_factor.use_clamp = True
    transparent = node_tree.nodes.new("ShaderNodeBsdfTransparent")
    transparent.location = transparent_location
    mix = node_tree.nodes.new("ShaderNodeMixShader")
    mix.location = mix_location

    node_tree.links.new(_node_socket(light_path.outputs, "Is Shadow Ray"), shadow_mix_factor.inputs[0])
    node_tree.links.new(disable_shadow_socket, shadow_mix_factor.inputs[1])
    node_tree.links.new(shadow_mix_factor.outputs[0], mix.inputs[0])
    node_tree.links.new(surface_socket, mix.inputs[1])
    node_tree.links.new(transparent.outputs[0], mix.inputs[2])
    node_tree.links.new(mix.outputs[0], output_socket)


def _palette_tinted_color_socket(
    node_tree: bpy.types.NodeTree,
    group_input: bpy.types.Node,
    texture_input_name: str,
    *,
    x: float,
    y: float,
) -> Any | None:
    current_socket = _group_input_socket(group_input, texture_input_name)
    palette_names = _palette_input_names(node_tree)
    if current_socket is None and not palette_names:
        return None
    if current_socket is None:
        current_socket = _white_color_socket(node_tree, (x, y))
    for index, socket_name in enumerate(palette_names):
        palette_socket = _group_input_socket(group_input, socket_name)
        if palette_socket is None:
            continue
        multiply = node_tree.nodes.new("ShaderNodeMixRGB")
        multiply.location = (x + 180.0 * (index + 1), y - 30.0 * index)
        multiply.blend_type = "MULTIPLY"
        multiply.inputs[0].default_value = 1.0
        node_tree.links.new(current_socket, multiply.inputs[1])
        node_tree.links.new(palette_socket, multiply.inputs[2])
        current_socket = multiply.outputs[0]
    return current_socket


def _author_hard_surface(node_tree: bpy.types.NodeTree) -> None:
    _ensure_palette_inputs(node_tree)
    _remove_input_socket(node_tree, "Alpha")
    base_alpha_name = _ensure_paired_alpha_input(
        node_tree,
        "TexSlot1_BaseColor",
        semantic="base_color",
        source_slot="TexSlot1",
    )
    _ensure_input_socket(
        node_tree,
        "Disable Shadow",
        "NodeSocketBool",
        semantic="disable_shadow",
        source_slot=None,
        required=False,
        default_value=False,
    )
    _ensure_input_socket(
        node_tree,
        "TexSlot10_IridescenceColor",
        "NodeSocketColor",
        semantic="iridescence_color",
        source_slot="TexSlot10",
        required=False,
        default_value=None,
    )
    _ensure_input_socket(
        node_tree,
        "Virtual_Roughness",
        "NodeSocketFloat",
        semantic="roughness",
        source_slot=None,
        required=False,
        default_value=0.45,
    )
    _reorder_input_sockets(
        node_tree,
        [
            "TexSlot1_BaseColor",
            base_alpha_name,
            "TexSlot14_Emissive",
            "TexSlot3_NormalGloss",
            "TexSlot6_Displacement",
            *_palette_input_names(node_tree),
            "Virtual_Roughness",
            "TexSlot10_IridescenceColor",
            "Disable Shadow",
        ],
    )
    group_input, group_output = _reset_group_nodes(node_tree)
    principled = node_tree.nodes.new("ShaderNodeBsdfPrincipled")
    principled.location = (-40.0, 0.0)

    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    _connect_disable_shadow(
        node_tree,
        group_input,
        principled.outputs["BSDF"],
        output_socket,
        mix_location=(360.0, 220.0),
        transparent_location=(140.0, 220.0),
        light_path_location=(-260.0, 220.0),
        math_location=(-60.0, 220.0),
    )

    base_color_socket = _node_socket(principled.inputs, "Base Color")
    base_color = _palette_tinted_color_socket(node_tree, group_input, "TexSlot1_BaseColor", x=-420.0, y=140.0)
    if base_color is not None and base_color_socket is not None:
        node_tree.links.new(base_color, base_color_socket)

    alpha_socket = _combine_value_sockets(
        node_tree,
        [_group_input_socket(group_input, base_alpha_name)],
        x=-300.0,
        y=260.0,
    )
    principled_alpha = _node_socket(principled.inputs, "Alpha")
    if alpha_socket is not None and principled_alpha is not None:
        node_tree.links.new(alpha_socket, principled_alpha)

    _connect_normal_map(node_tree, group_input, "TexSlot3_NormalGloss", _node_socket(principled.inputs, "Normal"))
    roughness_socket = _node_socket(principled.inputs, "Roughness")
    if roughness_socket is not None:
        roughness_socket.default_value = 0.45
    roughness_input = _group_input_socket(group_input, "Virtual_Roughness")
    if roughness_input is not None and roughness_socket is not None:
        node_tree.links.new(roughness_input, roughness_socket)

    emissive_socket = _group_input_socket(group_input, "TexSlot14_Emissive")
    emission_color = _node_socket(principled.inputs, "Emission Color", "Emission")
    if emissive_socket is not None and emission_color is not None:
        node_tree.links.new(emissive_socket, emission_color)
        emission_strength = _node_socket(principled.inputs, "Emission Strength")
        if emission_strength is not None:
            emission_strength.default_value = 1.0

    iridescence_socket = _group_input_socket(group_input, "TexSlot10_IridescenceColor")
    coat_tint = _node_socket(principled.inputs, "Coat Tint")
    if iridescence_socket is not None and coat_tint is not None:
        node_tree.links.new(iridescence_socket, coat_tint)
        coat_weight = _node_socket(principled.inputs, "Coat Weight")
        if coat_weight is not None:
            coat_weight.default_value = 0.35

    auto_layout_node_tree(node_tree)


def _author_glass(node_tree: bpy.types.NodeTree) -> None:
    _ensure_palette_inputs(node_tree)
    _ensure_input_socket(
        node_tree,
        "TexSlot11_Dirt",
        "NodeSocketColor",
        semantic="dirt",
        source_slot="TexSlot11",
        required=False,
        default_value=[1.0, 1.0, 1.0, 1.0],
    )
    _ensure_input_socket(
        node_tree,
        "TexSlot4_TintColor",
        "NodeSocketColor",
        semantic="tint_color",
        source_slot="TexSlot4",
        required=False,
        default_value=[1.0, 1.0, 1.0, 1.0],
    )
    _ensure_input_socket(
        node_tree,
        "Palette_Glass",
        "NodeSocketColor",
        semantic="palette_glass",
        source_slot=None,
        required=False,
        default_value=[1.0, 1.0, 1.0, 1.0],
    )
    _ensure_input_socket(
        node_tree,
        "TexSlot15_CondensationNormal",
        "NodeSocketColor",
        semantic="condensation_normal",
        source_slot="TexSlot15",
        required=False,
        default_value=None,
    )
    group_input, group_output = _reset_group_nodes(node_tree)
    glass = node_tree.nodes.new("ShaderNodeBsdfGlass")
    glass.location = (-40.0, 0.0)

    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    node_tree.links.new(glass.outputs[0], output_socket)

    roughness = _node_socket(glass.inputs, "Roughness")
    if roughness is not None:
        roughness.default_value = 0.03
    ior = _node_socket(glass.inputs, "IOR")
    if ior is not None:
        ior.default_value = 1.05

    tint_socket = _palette_tinted_color_socket(node_tree, group_input, "TexSlot4_TintColor", x=-420.0, y=120.0)
    dirt_socket = _group_input_socket(group_input, "TexSlot11_Dirt")
    base_color_socket = _node_socket(glass.inputs, "Color")
    glass_color = tint_socket or dirt_socket
    if tint_socket is not None and dirt_socket is not None:
        dirt_mix = node_tree.nodes.new("ShaderNodeMixRGB")
        dirt_mix.location = (-220.0, 120.0)
        dirt_mix.blend_type = "MULTIPLY"
        dirt_mix.inputs[0].default_value = 1.0
        node_tree.links.new(dirt_socket, dirt_mix.inputs[1])
        node_tree.links.new(tint_socket, dirt_mix.inputs[2])
        glass_color = dirt_mix.outputs[0]
    if glass_color is not None and base_color_socket is not None:
        node_tree.links.new(glass_color, base_color_socket)
    elif base_color_socket is not None:
        node_tree.links.new(_white_color_socket(node_tree, (-260.0, 120.0)), base_color_socket)

    wear_gloss_socket = _group_input_socket(group_input, "TexSlot6_WearGloss")
    if wear_gloss_socket is not None and roughness is not None:
        roughness_value = _rgb_to_bw(node_tree, wear_gloss_socket, (-260.0, -40.0))
        roughness_invert = node_tree.nodes.new("ShaderNodeMath")
        roughness_invert.location = (-60.0, -40.0)
        roughness_invert.operation = "SUBTRACT"
        roughness_invert.inputs[0].default_value = 1.0
        node_tree.links.new(roughness_value, roughness_invert.inputs[1])
        node_tree.links.new(roughness_invert.outputs[0], roughness)

    normal_source = _group_input_socket(group_input, "TexSlot2_NormalGloss")
    normal_target = _node_socket(glass.inputs, "Normal")
    if normal_source is not None and normal_target is not None:
        normal_map = node_tree.nodes.new("ShaderNodeNormalMap")
        normal_map.location = (-220.0, -220.0)
        normal_map.inputs["Strength"].default_value = 0.25
        node_tree.links.new(normal_source, normal_map.inputs["Color"])
        node_tree.links.new(normal_map.outputs["Normal"], normal_target)


def _author_display_screen(node_tree: bpy.types.NodeTree) -> None:
    screen_alpha_name = _ensure_paired_alpha_input(
        node_tree,
        "TexSlot9_ScreenSource",
        semantic="screen_source",
        source_slot="TexSlot9",
    )
    group_input, group_output = _reset_group_nodes(node_tree)
    emission = node_tree.nodes.new("ShaderNodeEmission")
    emission.location = (-40.0, 0.0)
    emission.inputs["Strength"].default_value = 3.0

    color_socket = _group_input_socket_by_semantic(group_input, node_tree, "screen_source", "base_color")
    if color_socket is None:
        color_socket = _first_group_color_socket(group_input, node_tree)
    if color_socket is not None:
        node_tree.links.new(color_socket, emission.inputs["Color"])

    factor_socket = _group_input_socket_by_semantic(group_input, node_tree, "screen_surface_mask", "display_mask", "pixel_layout")
    if factor_socket is not None:
        factor_socket = _rgb_to_bw(node_tree, factor_socket, (-220.0, -180.0))
    factor_socket = _combine_value_sockets(
        node_tree,
        [factor_socket, _group_input_socket(group_input, screen_alpha_name)],
        x=-40.0,
        y=-220.0,
    )
    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    _connect_surface_mix(
        node_tree,
        emission.outputs[0],
        output_socket,
        factor_socket=factor_socket,
        factor_default=0.12,
        mix_location=(260.0, 0.0),
        transparent_location=(20.0, -180.0),
    )


def _author_layer_blend(node_tree: bpy.types.NodeTree) -> None:
    _ensure_palette_inputs(node_tree)
    base_alpha_name = _ensure_paired_alpha_input(
        node_tree,
        "TexSlot1_BaseColor",
        semantic="base_color",
        source_slot="TexSlot1",
    )
    group_input, group_output = _reset_group_nodes(node_tree)
    principled = node_tree.nodes.new("ShaderNodeBsdfPrincipled")
    principled.location = (-40.0, 0.0)

    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    node_tree.links.new(principled.outputs["BSDF"], output_socket)

    base_color_socket = _node_socket(principled.inputs, "Base Color")
    base_color = _palette_tinted_color_socket(node_tree, group_input, "TexSlot1_BaseColor", x=-420.0, y=120.0)
    if base_color is None:
        base_color = _group_input_socket_by_semantic(group_input, node_tree, "base_color")
    if base_color is None:
        base_color = _first_group_color_socket(group_input, node_tree)
    if base_color is not None and base_color_socket is not None:
        node_tree.links.new(base_color, base_color_socket)

    base_alpha = _group_input_socket(group_input, base_alpha_name)
    principled_alpha = _node_socket(principled.inputs, "Alpha")
    if base_alpha is not None and principled_alpha is not None:
        node_tree.links.new(base_alpha, principled_alpha)

    roughness_socket = _node_socket(principled.inputs, "Roughness")
    if roughness_socket is not None:
        roughness_socket.default_value = 0.45
    roughness_source = _group_input_socket_by_semantic(group_input, node_tree, "wear_gloss", "roughness")
    if roughness_source is not None and roughness_socket is not None:
        roughness_value = _rgb_to_bw(node_tree, roughness_source, (-260.0, -60.0))
        node_tree.links.new(roughness_value, roughness_socket)

    _connect_standard_normal(node_tree, group_input, _node_socket(principled.inputs, "Normal"), "normal_gloss")


def _author_mesh_decal(node_tree: bpy.types.NodeTree) -> None:
    _remove_input_socket(node_tree, "Alpha")
    decal_alpha_name = _ensure_paired_alpha_input(
        node_tree,
        "TexSlot1_DecalSource",
        semantic="decal_source",
        source_slot="TexSlot1",
    )
    stencil_alpha_name = _ensure_paired_alpha_input(
        node_tree,
        "TexSlot7_StencilSource",
        semantic="stencil_source",
        source_slot="TexSlot7",
    )
    _ensure_input_socket(
        node_tree,
        "Disable Shadow",
        "NodeSocketBool",
        semantic="disable_shadow",
        source_slot=None,
        required=False,
        default_value=False,
    )
    # Phase 1 of decal-authoritative polish (Option A): expose the authored
    # MeshDecal public params so the importer can set them per submaterial.
    # Semantics carry the ``public_param_<name>`` prefix so the builder can
    # route them generically via ``_apply_public_param_defaults``.
    _ensure_input_socket(
        node_tree,
        "Param_DecalDiffuseOpacity",
        "NodeSocketFloat",
        semantic="public_param_decaldiffuseopacity",
        source_slot=None,
        required=False,
        default_value=1.0,
    )
    _ensure_input_socket(
        node_tree,
        "Param_DecalAlphaMultiplier",
        "NodeSocketFloat",
        semantic="public_param_decalalphamultiplier",
        source_slot=None,
        required=False,
        default_value=1.0,
    )
    _ensure_input_socket(
        node_tree,
        "Param_DecalFalloff",
        "NodeSocketFloat",
        semantic="public_param_decalfalloff",
        source_slot=None,
        required=False,
        default_value=1.0,
    )
    _ensure_input_socket(
        node_tree,
        "Param_ETintModeType",
        "NodeSocketFloat",
        semantic="public_param_etintmodetype",
        source_slot=None,
        required=False,
        default_value=0.0,
    )
    # Phase 2 wear-through: height-driven fade toward an authored wear
    # palette (diffuse + specular + glossiness). ``DamagePerObjectWear``
    # is the global amplitude — at 0.0 (the authored Aurora value) the
    # wear chain is fully gated off and the output is unchanged.
    _ensure_input_socket(
        node_tree,
        "Param_WearBlendBase",
        "NodeSocketFloat",
        semantic="public_param_wearblendbase",
        source_slot=None,
        required=False,
        default_value=0.0,
    )
    _ensure_input_socket(
        node_tree,
        "Param_WearBlendFalloff",
        "NodeSocketFloat",
        semantic="public_param_wearblendfalloff",
        source_slot=None,
        required=False,
        default_value=0.5,
    )
    _ensure_input_socket(
        node_tree,
        "Param_WearDiffuseColor",
        "NodeSocketColor",
        semantic="public_param_weardiffusecolor",
        source_slot=None,
        required=False,
        default_value=[1.0, 1.0, 1.0, 1.0],
    )
    _ensure_input_socket(
        node_tree,
        "Param_WearSpecularColor",
        "NodeSocketColor",
        semantic="public_param_wearspecularcolor",
        source_slot=None,
        required=False,
        default_value=[1.0, 1.0, 1.0, 1.0],
    )
    _ensure_input_socket(
        node_tree,
        "Param_WearGlossiness",
        "NodeSocketFloat",
        semantic="public_param_wearglossiness",
        source_slot=None,
        required=False,
        default_value=1.0,
    )
    _ensure_input_socket(
        node_tree,
        "Param_DamagePerObjectWear",
        "NodeSocketFloat",
        semantic="public_param_damageperobjectwear",
        source_slot=None,
        required=False,
        default_value=0.0,
    )
    # Option E: livery-aware host tint. The runtime builder wires this
    # from the package's palette group's ``Decal Color`` output when the
    # palette actually authors decal colour data; otherwise the default
    # white passes the decal through unchanged.
    _ensure_input_socket(
        node_tree,
        "Host Tint",
        "NodeSocketColor",
        semantic="host_tint",
        source_slot=None,
        required=False,
        default_value=[1.0, 1.0, 1.0, 1.0],
    )
    # Option E2-Lite metallic+roughness: per-host-channel palette tint for
    # the decal's specular reflectance and baseline roughness. Defaults
    # (white, 0.5) keep the graph a no-op until the runtime builder wires
    # these to the palette's ``<Channel> SpecColor`` and ``<Channel>
    # Glossiness`` outputs (inverted to roughness).
    _ensure_input_socket(
        node_tree,
        "Host Specular Tint",
        "NodeSocketColor",
        semantic="host_specular_tint",
        source_slot=None,
        required=False,
        default_value=[1.0, 1.0, 1.0, 1.0],
    )
    _ensure_input_socket(
        node_tree,
        "Host Roughness",
        "NodeSocketFloat",
        semantic="host_roughness",
        source_slot=None,
        required=False,
        default_value=0.5,
    )
    _reorder_input_sockets(
        node_tree,
        [
            "TexSlot1_DecalSource",
            decal_alpha_name,
            "TexSlot2_Specular",
            "TexSlot3_NormalGloss",
            "TexSlot4_Height",
            "TexSlot5_BreakupMask",
            "TexSlot6_TintMask",
            "TexSlot7_StencilSource",
            stencil_alpha_name,
            "TexSlot8_GrimeBreakup",
            "Param_DecalDiffuseOpacity",
            "Param_DecalAlphaMultiplier",
            "Param_DecalFalloff",
            "Param_ETintModeType",
            "Param_WearBlendBase",
            "Param_WearBlendFalloff",
            "Param_WearDiffuseColor",
            "Param_WearSpecularColor",
            "Param_WearGlossiness",
            "Param_DamagePerObjectWear",
            "Host Tint",
            "Host Specular Tint",
            "Host Roughness",
            "Disable Shadow",
        ],
    )
    group_input, group_output = _reset_group_nodes(node_tree)
    principled = node_tree.nodes.new("ShaderNodeBsdfPrincipled")
    principled.location = (-40.0, 40.0)

    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    # Decals and POM surfaces sit slightly above the hull they annotate;
    # glossy rays sampling them from a nearby reflecting surface produce
    # visible ghost offsets. Cull the surface for close glossy rays only
    # (<0.05m); distant reflections pass through unchanged.
    culled_surface = _cull_close_reflections(
        node_tree,
        principled.outputs["BSDF"],
        max_distance=0.05,
        location=(200.0, 220.0),
    )
    _connect_disable_shadow(
        node_tree,
        group_input,
        culled_surface,
        output_socket,
        mix_location=(360.0, 220.0),
        transparent_location=(140.0, 220.0),
        light_path_location=(-260.0, 220.0),
        math_location=(-60.0, 220.0),
    )

    base_color = _group_input_socket_by_semantic(group_input, node_tree, "decal_source", "stencil_source", "base_color")
    if base_color is None:
        base_color = _first_group_color_socket(group_input, node_tree)

    # Per-corner vertex color authored on decal meshes. On current fixtures
    # the R channel carries a breakup mask and the alpha channel carries an
    # edge-falloff factor; when the submaterial's ETintModeType selects the
    # vertex-color tint path (value 3 in the engine enum), the colour is
    # multiplied into the decal's diffuse as well.
    vertex_color = node_tree.nodes.new("ShaderNodeVertexColor")
    vertex_color.layer_name = "Color"
    vertex_color.location = (-780.0, -80.0)

    tint_mode_socket = _group_input_socket_by_semantic(
        group_input, node_tree, "public_param_etintmodetype"
    )
    tint_selector = node_tree.nodes.new("ShaderNodeMath")
    tint_selector.operation = "COMPARE"
    tint_selector.name = "Tint Mode 3 Select"
    tint_selector.label = "mode == 3"
    tint_selector.location = (-560.0, 40.0)
    tint_selector.inputs[1].default_value = 3.0
    tint_selector.inputs[2].default_value = 0.5  # epsilon for integer compare
    if tint_mode_socket is not None:
        node_tree.links.new(tint_mode_socket, tint_selector.inputs[0])

    tinted_diffuse = node_tree.nodes.new("ShaderNodeMix")
    tinted_diffuse.data_type = "RGBA"
    tinted_diffuse.name = "Apply VC Tint"
    tinted_diffuse.label = "vc tint"
    tinted_diffuse.blend_type = "MULTIPLY"
    tinted_diffuse.clamp_factor = True
    tinted_diffuse.location = (-360.0, 0.0)
    node_tree.links.new(tint_selector.outputs[0], tinted_diffuse.inputs[0])
    if base_color is not None:
        node_tree.links.new(base_color, tinted_diffuse.inputs[6])  # A (RGBA)
    node_tree.links.new(vertex_color.outputs["Color"], tinted_diffuse.inputs[7])  # B (RGBA)

    # Phase 2 wear mask: smoothstep(WearBlendBase, WearBlendBase +
    # WearBlendFalloff, 1 - height.R) gated by DamagePerObjectWear.
    # Authored ``WearBlend*`` ranges are in height space; inverting the
    # height makes lower regions read as "worn" which matches CryEngine's
    # convention for MeshDecal's per-object damage channel.
    height_socket = _group_input_socket_by_semantic(group_input, node_tree, "height")
    wear_blend_base = _group_input_socket_by_semantic(group_input, node_tree, "public_param_wearblendbase")
    wear_blend_falloff = _group_input_socket_by_semantic(group_input, node_tree, "public_param_wearblendfalloff")
    wear_amount = _group_input_socket_by_semantic(group_input, node_tree, "public_param_damageperobjectwear")
    wear_mask: Any | None = None
    if height_socket is not None:
        sep_height = node_tree.nodes.new("ShaderNodeSeparateColor")
        sep_height.mode = "RGB"
        sep_height.location = (-780.0, 260.0)
        node_tree.links.new(height_socket, sep_height.inputs[0])

        inv_height = node_tree.nodes.new("ShaderNodeMath")
        inv_height.operation = "SUBTRACT"
        inv_height.use_clamp = True
        inv_height.label = "1 - height"
        inv_height.location = (-600.0, 260.0)
        inv_height.inputs[0].default_value = 1.0
        node_tree.links.new(sep_height.outputs["Red"], inv_height.inputs[1])

        wear_range_max = node_tree.nodes.new("ShaderNodeMath")
        wear_range_max.operation = "ADD"
        wear_range_max.use_clamp = True
        wear_range_max.label = "base + falloff"
        wear_range_max.location = (-600.0, 360.0)
        if wear_blend_base is not None:
            node_tree.links.new(wear_blend_base, wear_range_max.inputs[0])
        if wear_blend_falloff is not None:
            node_tree.links.new(wear_blend_falloff, wear_range_max.inputs[1])

        wear_smooth = node_tree.nodes.new("ShaderNodeMapRange")
        wear_smooth.interpolation_type = "SMOOTHSTEP"
        wear_smooth.clamp = True
        wear_smooth.label = "wear smoothstep"
        wear_smooth.location = (-420.0, 360.0)
        node_tree.links.new(inv_height.outputs[0], wear_smooth.inputs[0])  # Value
        if wear_blend_base is not None:
            node_tree.links.new(wear_blend_base, wear_smooth.inputs[1])  # From Min
        node_tree.links.new(wear_range_max.outputs[0], wear_smooth.inputs[2])  # From Max
        wear_smooth.inputs[3].default_value = 0.0  # To Min
        wear_smooth.inputs[4].default_value = 1.0  # To Max

        wear_gate = node_tree.nodes.new("ShaderNodeMath")
        wear_gate.operation = "MULTIPLY"
        wear_gate.use_clamp = True
        wear_gate.label = "wear × damage"
        wear_gate.location = (-240.0, 360.0)
        node_tree.links.new(wear_smooth.outputs[0], wear_gate.inputs[0])
        if wear_amount is not None:
            node_tree.links.new(wear_amount, wear_gate.inputs[1])
        else:
            wear_gate.inputs[1].default_value = 0.0
        wear_mask = wear_gate.outputs[0]

    # Apply wear to base color: mix(tinted_diffuse, WearDiffuseColor, wear).
    base_color_final = tinted_diffuse.outputs[2]
    wear_diffuse = _group_input_socket_by_semantic(group_input, node_tree, "public_param_weardiffusecolor")
    if wear_mask is not None and wear_diffuse is not None:
        wear_mix = node_tree.nodes.new("ShaderNodeMix")
        wear_mix.data_type = "RGBA"
        wear_mix.name = "Apply Wear Diffuse"
        wear_mix.label = "wear diffuse"
        wear_mix.blend_type = "MIX"
        wear_mix.clamp_factor = True
        wear_mix.location = (-140.0, 0.0)
        node_tree.links.new(wear_mask, wear_mix.inputs[0])
        node_tree.links.new(tinted_diffuse.outputs[2], wear_mix.inputs[6])  # A
        node_tree.links.new(wear_diffuse, wear_mix.inputs[7])  # B
        base_color_final = wear_mix.outputs[2]

    # Option E: multiply by livery-driven ``Host Tint``. Default white so
    # this is a no-op until the runtime builder links it to the package
    # palette's ``Decal Color`` output.
    host_tint = _group_input_socket_by_semantic(group_input, node_tree, "host_tint")
    if host_tint is not None:
        host_mix = node_tree.nodes.new("ShaderNodeMix")
        host_mix.data_type = "RGBA"
        host_mix.name = "Apply Host Tint"
        host_mix.label = "host tint"
        host_mix.blend_type = "MULTIPLY"
        host_mix.clamp_factor = True
        host_mix.inputs[0].default_value = 1.0
        host_mix.location = (60.0, 0.0)
        node_tree.links.new(base_color_final, host_mix.inputs[6])  # A
        node_tree.links.new(host_tint, host_mix.inputs[7])  # B
        base_color_final = host_mix.outputs[2]

    base_color_socket = _node_socket(principled.inputs, "Base Color")
    if base_color_socket is not None:
        node_tree.links.new(base_color_final, base_color_socket)

    # Combined alpha chain: DecalSource.alpha × DecalDiffuseOpacity ×
    # DecalAlphaMultiplier × DecalFalloff.
    #
    # Deliberately excluded:
    #   * StencilSource_alpha — stencil is an optional diffuse overlay, not
    #     a global alpha gate; on materials without a stencil texture the
    #     socket is unlinked at 0 and would zero the whole chain.
    #   * VertexColor.Alpha — semantics unconfirmed across decal meshes;
    #     fixture evidence is limited and zero-valued vertex alpha would
    #     collapse visibility.
    alpha_terms = [
        _group_input_socket(group_input, decal_alpha_name),
        _group_input_socket_by_semantic(group_input, node_tree, "public_param_decaldiffuseopacity"),
        _group_input_socket_by_semantic(group_input, node_tree, "public_param_decalalphamultiplier"),
        _group_input_socket_by_semantic(group_input, node_tree, "public_param_decalfalloff"),
    ]
    alpha_terms = [term for term in alpha_terms if term is not None]

    alpha_chain: Any = None
    for index, term in enumerate(alpha_terms):
        if alpha_chain is None:
            alpha_chain = term
            continue
        mul = node_tree.nodes.new("ShaderNodeMath")
        mul.operation = "MULTIPLY"
        mul.location = (-360.0 + 60.0 * index, 260.0)
        mul.label = "alpha mul"
        node_tree.links.new(alpha_chain, mul.inputs[0])
        node_tree.links.new(term, mul.inputs[1])
        alpha_chain = mul.outputs[0]

    principled_alpha = _node_socket(principled.inputs, "Alpha")
    if alpha_chain is not None and principled_alpha is not None:
        node_tree.links.new(alpha_chain, principled_alpha)

    # Specular: mix(bw(Specular), bw(WearSpecularColor), wear_mask).
    specular_source = _group_input_socket_by_semantic(group_input, node_tree, "specular")
    specular_target = _node_socket(principled.inputs, "Specular IOR Level", "Specular")
    wear_specular = _group_input_socket_by_semantic(group_input, node_tree, "public_param_wearspecularcolor")
    host_spec_tint = _group_input_socket_by_semantic(group_input, node_tree, "host_specular_tint")
    if specular_source is not None and specular_target is not None:
        specular_bw = _rgb_to_bw(node_tree, specular_source, (-260.0, -260.0))
        if wear_mask is not None and wear_specular is not None:
            wear_spec_bw = _rgb_to_bw(node_tree, wear_specular, (-260.0, -340.0))
            spec_wear_mix = node_tree.nodes.new("ShaderNodeMix")
            spec_wear_mix.data_type = "FLOAT"
            spec_wear_mix.clamp_factor = True
            spec_wear_mix.label = "wear spec"
            spec_wear_mix.location = (-80.0, -300.0)
            node_tree.links.new(wear_mask, spec_wear_mix.inputs[0])
            node_tree.links.new(specular_bw, spec_wear_mix.inputs[2])  # A (float)
            node_tree.links.new(wear_spec_bw, spec_wear_mix.inputs[3])  # B (float)
            spec_result = spec_wear_mix.outputs[0]
        else:
            spec_result = specular_bw
        # Option E2-Lite metallic+roughness: scale by bw(Host Specular
        # Tint). Default white keeps this a no-op.
        if host_spec_tint is not None:
            host_spec_bw = _rgb_to_bw(node_tree, host_spec_tint, (-260.0, -380.0))
            host_spec_mul = node_tree.nodes.new("ShaderNodeMath")
            host_spec_mul.operation = "MULTIPLY"
            host_spec_mul.use_clamp = True
            host_spec_mul.label = "host spec"
            host_spec_mul.location = (-40.0, -380.0)
            node_tree.links.new(spec_result, host_spec_mul.inputs[0])
            node_tree.links.new(host_spec_bw, host_spec_mul.inputs[1])
            spec_result = host_spec_mul.outputs[0]
        node_tree.links.new(spec_result, specular_target)

    # Roughness: mix(Host Roughness, 1 - WearGlossiness, wear_mask).
    wear_gloss = _group_input_socket_by_semantic(group_input, node_tree, "public_param_wearglossiness")
    host_rough = _group_input_socket_by_semantic(group_input, node_tree, "host_roughness")
    roughness_target = _node_socket(principled.inputs, "Roughness")
    if wear_mask is not None and wear_gloss is not None and roughness_target is not None:
        inv_gloss = node_tree.nodes.new("ShaderNodeMath")
        inv_gloss.operation = "SUBTRACT"
        inv_gloss.use_clamp = True
        inv_gloss.label = "1 - glossiness"
        inv_gloss.location = (-260.0, -420.0)
        inv_gloss.inputs[0].default_value = 1.0
        node_tree.links.new(wear_gloss, inv_gloss.inputs[1])

        rough_mix = node_tree.nodes.new("ShaderNodeMix")
        rough_mix.data_type = "FLOAT"
        rough_mix.clamp_factor = True
        rough_mix.label = "wear rough"
        rough_mix.location = (-80.0, -420.0)
        node_tree.links.new(wear_mask, rough_mix.inputs[0])
        # Option E2-Lite metallic+roughness: baseline (unworn) roughness
        # is driven by the palette's per-channel Glossiness via the
        # runtime builder; default 0.5 when unwired.
        if host_rough is not None:
            node_tree.links.new(host_rough, rough_mix.inputs[2])  # A
        else:
            rough_mix.inputs[2].default_value = 0.5
        node_tree.links.new(inv_gloss.outputs[0], rough_mix.inputs[3])  # B
        node_tree.links.new(rough_mix.outputs[0], roughness_target)
    elif host_rough is not None and roughness_target is not None:
        # No wear data, but we still want Host Roughness to drive the
        # base roughness directly.
        node_tree.links.new(host_rough, roughness_target)

    _connect_standard_normal(node_tree, group_input, _node_socket(principled.inputs, "Normal"), "normal_gloss")

    auto_layout_node_tree(node_tree)


def _author_ui_mesh(node_tree: bpy.types.NodeTree) -> None:
    group_input, group_output = _reset_group_nodes(node_tree)
    emission = node_tree.nodes.new("ShaderNodeEmission")
    emission.location = (-40.0, 0.0)
    emission.inputs["Color"].default_value = (0.0, 1.0, 1.0, 1.0)
    emission.inputs["Strength"].default_value = 2.5
    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    _connect_surface_mix(
        node_tree,
        emission.outputs[0],
        output_socket,
        factor_socket=None,
        factor_default=0.35,
        mix_location=(260.0, 0.0),
        transparent_location=(20.0, -180.0),
    )


def _author_illum(node_tree: bpy.types.NodeTree) -> None:
    _ensure_palette_inputs(node_tree)
    _remove_input_socket(node_tree, "Alpha")
    base_alpha_name = _ensure_paired_alpha_input(
        node_tree,
        "TexSlot1_BaseColor",
        semantic="base_color",
        source_slot="TexSlot1",
    )
    _ensure_input_socket(
        node_tree,
        "Disable Shadow",
        "NodeSocketBool",
        semantic="disable_shadow",
        source_slot=None,
        required=False,
        default_value=False,
    )
    _ensure_input_socket(
        node_tree,
        "Emission Strength",
        "NodeSocketFloat",
        semantic="emission_strength",
        source_slot=None,
        required=False,
        default_value=0.0,
    )
    _reorder_input_sockets(
        node_tree,
        [
            "TexSlot1_BaseColor",
            base_alpha_name,
            "TexSlot10_SpecularSecondary",
            "TexSlot11_HeightSecondary",
            "TexSlot12_BlendMask",
            "TexSlot13_DetailSecondary",
            "TexSlot2_NormalGlossPrimary",
            "TexSlot3_NormalGlossSecondary",
            "TexSlot4_Specular",
            "TexSlot6_DetailAux",
            "TexSlot8_Height",
            "TexSlot9_BaseColorSecondary",
            "Disable Shadow",
            "Emission Strength",
        ],
    )
    group_input, group_output = _reset_group_nodes(node_tree)
    principled = node_tree.nodes.new("ShaderNodeBsdfPrincipled")
    principled.location = (-80.0, 80.0)
    emission = node_tree.nodes.new("ShaderNodeEmission")
    emission.location = (-80.0, -120.0)
    add_shader = node_tree.nodes.new("ShaderNodeAddShader")
    add_shader.location = (140.0, -20.0)

    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    _connect_disable_shadow(
        node_tree,
        group_input,
        add_shader.outputs[0],
        output_socket,
        mix_location=(360.0, 260.0),
        transparent_location=(140.0, 260.0),
        light_path_location=(-260.0, 260.0),
        math_location=(-60.0, 260.0),
    )
    node_tree.links.new(principled.outputs["BSDF"], add_shader.inputs[0])
    node_tree.links.new(emission.outputs["Emission"], add_shader.inputs[1])

    base_color = _palette_tinted_color_socket(node_tree, group_input, "TexSlot1_BaseColor", x=-440.0, y=120.0)
    base_color_socket = _node_socket(principled.inputs, "Base Color")
    if base_color is not None and base_color_socket is not None:
        node_tree.links.new(base_color, base_color_socket)
    emissive_color = _group_input_socket_by_semantic(group_input, node_tree, "emissive") or base_color
    if emissive_color is not None:
        node_tree.links.new(emissive_color, emission.inputs["Color"])
    emission_strength = _group_input_socket(group_input, "Emission Strength")
    if emission_strength is not None:
        node_tree.links.new(emission_strength, emission.inputs["Strength"])
    else:
        emission.inputs["Strength"].default_value = 0.0

    alpha_socket = _combine_value_sockets(
        node_tree,
        [_group_input_socket(group_input, base_alpha_name)],
        x=-320.0,
        y=320.0,
    )
    principled_alpha = _node_socket(principled.inputs, "Alpha")
    if alpha_socket is not None and principled_alpha is not None:
        node_tree.links.new(alpha_socket, principled_alpha)

    _connect_normal_map(node_tree, group_input, "TexSlot2_NormalGlossPrimary", _node_socket(principled.inputs, "Normal"))

    specular_socket = _group_input_socket(group_input, "TexSlot4_Specular")
    specular_target = _node_socket(principled.inputs, "Specular IOR Level", "Specular")
    if specular_socket is not None and specular_target is not None:
        specular_value = _rgb_to_bw(node_tree, specular_socket, (-300.0, -260.0))
        node_tree.links.new(specular_value, specular_target)

    subsurface_socket = _group_input_socket(group_input, "TexSlot17_SubsurfaceMask")
    subsurface_target = _node_socket(principled.inputs, "Subsurface Weight", "Subsurface")
    if subsurface_socket is not None and subsurface_target is not None:
        subsurface_value = _rgb_to_bw(node_tree, subsurface_socket, (-300.0, -360.0))
        node_tree.links.new(subsurface_value, subsurface_target)


def _author_monitor(node_tree: bpy.types.NodeTree) -> None:
    base_alpha_name = _ensure_paired_alpha_input(
        node_tree,
        "TexSlot1_BaseColor",
        semantic="base_color",
        source_slot="TexSlot1",
    )
    group_input, group_output = _reset_group_nodes(node_tree)
    emission = node_tree.nodes.new("ShaderNodeEmission")
    emission.location = (-80.0, 0.0)
    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    _connect_surface_mix(
        node_tree,
        emission.outputs["Emission"],
        output_socket,
        factor_socket=_group_input_socket(group_input, base_alpha_name),
        factor_default=0.0,
        mix_location=(220.0, 0.0),
        transparent_location=(0.0, -180.0),
    )

    source_socket = _group_input_socket(group_input, "TexSlot1_BaseColor")
    if source_socket is not None:
        node_tree.links.new(source_socket, emission.inputs["Color"])
    emission.inputs["Strength"].default_value = 1.0


def _ensure_nodraw_group() -> bpy.types.NodeTree:
    node_tree = bpy.data.node_groups.get("SB_NoDraw_v1")
    if node_tree is None:
        node_tree = create_group_from_contract(
            {
                "name": "SB_NoDraw_v1",
                "shader_families": ["NoDraw"],
                "version": 1,
                "shader_output": "Shader",
                "inputs": [],
                "metadata": {
                    "status": "phase2_authored",
                    "note": "Transparent no-draw group authored directly in the Blender library.",
                },
            }
        )
    node_tree.use_fake_user = True
    return node_tree


def _author_nodraw(node_tree: bpy.types.NodeTree) -> None:
    _ensure_input_socket(
        node_tree,
        "Alpha",
        "NodeSocketFloat",
        semantic="alpha",
        source_slot="TexSlot1",
        required=False,
        default_value=0.0,
    )
    _ensure_input_socket(
        node_tree,
        "Disable Shadow",
        "NodeSocketBool",
        semantic="disable_shadow",
        source_slot=None,
        required=False,
        default_value=False,
    )
    _group_input, group_output = _reset_group_nodes(node_tree)
    transparent = node_tree.nodes.new("ShaderNodeBsdfTransparent")
    transparent.location = (-40.0, 0.0)
    output_socket = _node_socket(group_output.inputs, str(node_tree.get(SHADER_OUTPUT_KEY, "Shader")))
    node_tree.links.new(transparent.outputs[0], output_socket)


def author_phase2_core_groups() -> dict[str, Any]:
    authored: list[str] = []
    authors = {
        "SB_DisplayScreen_v1": _author_display_screen,
        "SB_GlassPBR_v1": _author_glass,
        "SB_HardSurface_v1": _author_hard_surface,
        "SB_Illum_v1": _author_illum,
        "SB_LayerBlend_V2_v1": _author_layer_blend,
        "SB_MeshDecal_v1": _author_mesh_decal,
        "SB_NoDraw_v1": _author_nodraw,
        "SB_UIMesh_v1": _author_ui_mesh,
        "SB_UIPlane_v1": _author_display_screen,
        "SB_Unknown_v1": _author_monitor,
    }
    _ensure_nodraw_group()
    for group_name, author in authors.items():
        node_tree = _ensure_group_from_contract(group_name)
        if node_tree is None:
            continue
        author(node_tree)
        authored.append(group_name)
    if bpy.data.filepath:
        bpy.ops.wm.save_mainfile()
    return {
        "current_file": bpy.data.filepath,
        "authored_groups": authored,
    }
