"""Phase 23E generic-code audit guards.

These checks keep runtime importer/package ops code free of ship-specific
string literals in executable code paths.
"""

from __future__ import annotations

import ast
import sys
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))

FORBIDDEN_TOKENS = (
    "scorpius",
    "clipper",
    "aurora",
    "talon",
    "vulture",
    "mole",
    "defender",
    "drak",
    "banu",
)

TARGET_FILES = (
    ADDON_ROOT / "starbreaker_addon" / "runtime" / "package_ops.py",
    ADDON_ROOT / "starbreaker_addon" / "runtime" / "importer" / "orchestration.py",
    ADDON_ROOT / "starbreaker_addon" / "runtime" / "importer" / "utils.py",
)


class TestGenericCodeAudit(unittest.TestCase):
    def test_runtime_files_avoid_ship_specific_string_literals(self) -> None:
        violations: list[str] = []

        for path in TARGET_FILES:
            source = path.read_text(encoding="utf-8")
            tree = ast.parse(source, filename=str(path))
            for node in ast.walk(tree):
                if not isinstance(node, ast.Constant) or not isinstance(node.value, str):
                    continue
                lowered = node.value.lower()
                for token in FORBIDDEN_TOKENS:
                    if token in lowered:
                        violations.append(f"{path.name}:{getattr(node, 'lineno', '?')} contains '{token}'")

        self.assertEqual(violations, [], "\n".join(violations))


if __name__ == "__main__":
    unittest.main()
