from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from material_template_library import author_phase2_core_groups


if __name__ == "__main__":
    result = author_phase2_core_groups()
    print(json.dumps(result, indent=2, sort_keys=True))