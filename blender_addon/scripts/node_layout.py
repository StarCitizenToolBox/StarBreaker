from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class LayoutConfig:
    start_x: float = -900.0
    start_y: float = 220.0
    column_gap: float = 260.0
    node_gap: float = 60.0
    lane_gap: float = 120.0
    lane_split_threshold: float = 180.0


@dataclass(frozen=True)
class LayoutNodeSpec:
    node: Any
    column: int
    original_y: float


@dataclass(frozen=True)
class LayoutPlan:
    specs: list[LayoutNodeSpec]


def _node_height(node: Any) -> float:
    dimensions = getattr(node, "dimensions", None)
    height = getattr(dimensions, "y", 0.0) if dimensions is not None else 0.0
    if height and height > 1.0:
        return float(height)
    if getattr(node, "bl_idname", "") == "ShaderNodeGroup":
        return 220.0
    if getattr(node, "bl_idname", "") in {"NodeGroupInput", "NodeGroupOutput"}:
        return 180.0
    return 160.0


def _layout_nodes(node_tree: Any) -> list[Any]:
    return [node for node in node_tree.nodes if getattr(node, "parent", None) is None]


def build_layout_plan(node_tree: Any) -> LayoutPlan:
    nodes = _layout_nodes(node_tree)
    columns = {node: 0 for node in nodes}

    changed = True
    while changed:
        changed = False
        for link in node_tree.links:
            from_node = getattr(link, "from_node", None)
            to_node = getattr(link, "to_node", None)
            if from_node not in columns or to_node not in columns:
                continue
            candidate = columns[from_node] + 1
            if candidate > columns[to_node]:
                columns[to_node] = candidate
                changed = True

    specs = [
        LayoutNodeSpec(node=node, column=columns[node], original_y=float(getattr(node.location, "y", 0.0)))
        for node in nodes
    ]
    return LayoutPlan(specs=specs)


def apply_group_layout(node_tree: Any, plan: LayoutPlan, config: LayoutConfig | None = None) -> None:
    cfg = config or LayoutConfig()
    specs_by_column: dict[int, list[LayoutNodeSpec]] = {}
    for spec in plan.specs:
        specs_by_column.setdefault(spec.column, []).append(spec)

    for column, specs in sorted(specs_by_column.items()):
        specs.sort(key=lambda spec: (-spec.original_y, getattr(spec.node, "name", "")))
        current_y = cfg.start_y
        previous_spec: LayoutNodeSpec | None = None
        previous_height = 0.0
        for spec in specs:
            node = spec.node
            height = _node_height(node)
            if previous_spec is None:
                y = current_y
            else:
                extra_gap = cfg.lane_gap if abs(previous_spec.original_y - spec.original_y) >= cfg.lane_split_threshold else 0.0
                current_y -= (previous_height * 0.5) + (height * 0.5) + cfg.node_gap + extra_gap
                y = current_y
            node.location = (cfg.start_x + column * cfg.column_gap, y)
            previous_spec = spec
            previous_height = height

    _align_group_endpoints(node_tree)
    _resolve_column_collisions(node_tree, cfg)


def _average_linked_y(node_tree: Any, node: Any, *, use_outputs: bool) -> float | None:
    linked_y: list[float] = []
    for link in node_tree.links:
        if use_outputs and getattr(link, "from_node", None) == node:
            linked_y.append(float(getattr(link.to_node.location, "y", 0.0)))
        if not use_outputs and getattr(link, "to_node", None) == node:
            linked_y.append(float(getattr(link.from_node.location, "y", 0.0)))
    if not linked_y:
        return None
    return sum(linked_y) / len(linked_y)


def _align_group_endpoints(node_tree: Any) -> None:
    for node in _layout_nodes(node_tree):
        bl_idname = getattr(node, "bl_idname", "")
        if bl_idname == "NodeGroupInput":
            average_y = _average_linked_y(node_tree, node, use_outputs=True)
            if average_y is not None:
                node.location = (node.location.x, average_y)
        elif bl_idname == "NodeGroupOutput":
            average_y = _average_linked_y(node_tree, node, use_outputs=False)
            if average_y is not None:
                node.location = (node.location.x, average_y)


def _resolve_column_collisions(node_tree: Any, config: LayoutConfig) -> None:
    columns: dict[float, list[Any]] = {}
    for node in _layout_nodes(node_tree):
        columns.setdefault(float(node.location.x), []).append(node)

    for nodes in columns.values():
        nodes.sort(key=lambda node: (-float(node.location.y), getattr(node, "name", "")))
        previous_node: Any | None = None
        previous_bottom = 0.0
        for node in nodes:
            height = _node_height(node)
            top = float(node.location.y)
            bottom = top - height
            if previous_node is not None:
                allowed_top = previous_bottom - config.node_gap
                if top > allowed_top:
                    top = allowed_top
                    bottom = top - height
                    node.location = (node.location.x, top)
            previous_node = node
            previous_bottom = bottom


def auto_layout_node_tree(node_tree: Any, config: LayoutConfig | None = None) -> LayoutPlan:
    plan = build_layout_plan(node_tree)
    apply_group_layout(node_tree, plan, config=config)
    return plan