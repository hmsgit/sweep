"""House style fixture."""

from enum import Enum
from typing import ParamSpec, TypeVar

T = TypeVar("T")
P = ParamSpec("P")
TIMEOUT = 5
RETRIES: int = 3  # tuned ✓
TOGGLE = False
TOGGLE = True
identity_key = "identity"
retry_delays = (0.1, 0.5)
negative_floor = -1
banner_text = f"{TIMEOUT} seconds"
mutable_registry = []
counter = 0
counter = 1


class Color(Enum):
    RED = "RED"
    green = "green"


def load():
    options = {"depth": 2, "flags": {"a": 1}}
    weird = {"not-ident": 1}
    merged = {**options, "depth": 5}
    combo = {"mode": 1, **options, "extra": 2}
    banner = "🎉 launched"
    return options, weird, merged, combo, banner
