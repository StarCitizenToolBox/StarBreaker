from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from material_template_library import validate_library


if __name__ == "__main__":
    failures = validate_library()
    print(
        json.dumps(
            {
                "failure_count": len(failures),
                "failures": [{"group_name": failure.group_name, "message": failure.message} for failure in failures],
            },
            indent=2,
            sort_keys=True,
        )
    )
    raise SystemExit(1 if failures else 0)