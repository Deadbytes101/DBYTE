import os
import re

FROZEN_BASE_HEAD = "a1a5c2458065f6f295d1c3e77df62a34846226e3"


def expected_head():
    value = os.environ.get("DBYTE_V13_EXPECTED_HEAD", "").strip().lower()
    if not value:
        return FROZEN_BASE_HEAD
    if re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise RuntimeError("V13_EXPECTED_HEAD_INVALID")
    return value
