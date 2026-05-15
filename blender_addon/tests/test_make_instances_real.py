from __future__ import annotations

import ast
import types
import unittest
from pathlib import Path


ADDON_ROOT = Path(__file__).resolve().parents[1]


def _load_ui_functions(*names: str):
    ui_path = ADDON_ROOT / "starbreaker_addon" / "ui.py"
    source = ui_path.read_text(encoding="utf-8")
    tree = ast.parse(source)
    namespace: dict = {
        "bpy": types.SimpleNamespace(
            types=types.SimpleNamespace(Context=object, Object=object),
            ops=types.SimpleNamespace(
                object=types.SimpleNamespace(
                    select_all=lambda **_kwargs: None,
                    duplicates_make_real=lambda **_kwargs: None,
                )
            ),
        ),
    }
    pending = set(names)
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name in pending:
            func_source = ast.get_source_segment(source, node)
            if func_source:
                exec(compile(ast.parse(func_source), str(ui_path), "exec"), namespace)  # noqa: S102
                pending.remove(node.name)
                if not pending:
                    break
    return tuple(namespace[name] for name in names), namespace


class _FakeObject:
    def __init__(self, name: str, *, is_instance: bool = False, data=None):
        self.name = name
        self.parent = None
        self.children: list[_FakeObject] = []
        self.instance_collection = object() if is_instance else None
        self.instance_type = "COLLECTION" if is_instance else "NONE"
        self.selected = False
        self.data = data

    def select_set(self, value: bool) -> None:
        self.selected = value

    @property
    def children_recursive(self) -> list[_FakeObject]:
        result: list[_FakeObject] = []
        stack = list(self.children)
        while stack:
            child = stack.pop()
            result.append(child)
            stack.extend(child.children)
        return result


class _FakeData:
    def __init__(self, name: str, *, linked: bool):
        self.name = name
        self.library = object() if linked else None
        self.copy_count = 0

    def copy(self):
        self.copy_count += 1
        return _FakeData(f"{self.name}_local", linked=False)


class TestMakeInstancesReal(unittest.TestCase):
    def test_collection_instance_objects_finds_root_and_descendants(self) -> None:
        (collect_instances,), _namespace = _load_ui_functions("_collection_instance_objects")
        root = _FakeObject("Root", is_instance=True)
        child = _FakeObject("Child")
        grandchild = _FakeObject("Grandchild", is_instance=True)
        child.parent = root
        grandchild.parent = child
        child.children.append(grandchild)
        root.children.append(child)

        instances = collect_instances(root)

        self.assertEqual([obj.name for obj in instances], ["Root", "Grandchild"])

    def test_make_package_collection_instances_real_recurses_nested_instances(self) -> None:
        (collect_instances, make_instances_real), namespace = _load_ui_functions(
            "_collection_instance_objects",
            "_make_package_collection_instances_real",
        )
        namespace["_collection_instance_objects"] = collect_instances
        root = _FakeObject("Root")
        first_instance = _FakeObject("First", is_instance=True)
        first_instance.parent = root
        root.children.append(first_instance)
        nested_instance = _FakeObject("Nested", is_instance=True)
        select_calls: list[str] = []
        duplicate_calls: list[dict[str, object]] = []

        def _select_all(*, action: str) -> None:
            select_calls.append(action)

        def _duplicates_make_real(**kwargs) -> None:
            duplicate_calls.append(kwargs)
            if len(duplicate_calls) == 1:
                nested_instance.parent = root
                root.children.append(nested_instance)

        namespace["bpy"].ops.object.select_all = _select_all
        namespace["bpy"].ops.object.duplicates_make_real = _duplicates_make_real
        active_holder = types.SimpleNamespace(active=None)
        context = types.SimpleNamespace(view_layer=types.SimpleNamespace(objects=active_holder))

        processed = make_instances_real(context, root, chunk_size=1)

        self.assertEqual(processed, 2)
        self.assertEqual(select_calls, ["DESELECT", "DESELECT"])
        self.assertEqual(
            duplicate_calls,
            [
                {"use_base_parent": True, "use_hierarchy": True},
                {"use_base_parent": True, "use_hierarchy": True},
            ],
        )
        self.assertIsNone(first_instance.instance_collection)
        self.assertEqual(first_instance.instance_type, "NONE")
        self.assertIsNone(nested_instance.instance_collection)
        self.assertEqual(nested_instance.instance_type, "NONE")
        self.assertIs(active_holder.active, nested_instance)

    def test_make_package_linked_object_data_local_reuses_one_copy_per_source_datablock(self) -> None:
        (linked_objects, make_linked_local), namespace = _load_ui_functions(
            "_linked_object_data_objects",
            "_make_package_linked_object_data_local",
        )
        namespace["_linked_object_data_objects"] = linked_objects
        shared_data = _FakeData("Shared", linked=True)
        local_data = _FakeData("Local", linked=False)
        root = _FakeObject("Root")
        first = _FakeObject("First", data=shared_data)
        second = _FakeObject("Second", data=shared_data)
        third = _FakeObject("Third", data=local_data)
        first.parent = root
        second.parent = root
        third.parent = root
        root.children.extend([first, second, third])

        localized = make_linked_local(root)

        self.assertEqual(localized, 1)
        self.assertEqual(shared_data.copy_count, 1)
        self.assertIs(first.data, second.data)
        self.assertIsNone(first.data.library)
        self.assertIs(third.data, local_data)


if __name__ == "__main__":
    unittest.main()
