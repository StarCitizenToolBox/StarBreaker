from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from material_template_library import export_contract


if __name__ == "__main__":
    result = export_contract()
    print(json.dumps({"group_count": len(result.get("groups", []))}, indent=2, sort_keys=True))
