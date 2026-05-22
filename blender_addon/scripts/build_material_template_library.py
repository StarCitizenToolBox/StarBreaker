from __future__ import annotations

import json

from material_template_library import build_library


if __name__ == "__main__":
    result = build_library()
    print(json.dumps(result, indent=2, sort_keys=True))
