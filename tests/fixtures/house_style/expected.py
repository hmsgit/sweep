"""House style fixture."""

from typing import Final
from enum import Enum
from typing import ParamSpec, TypeVar

T = TypeVar("T")
P = ParamSpec("P")
TIMEOUT: Final = 5
RETRIES: int = 3  # tuned
TOGGLE = False
TOGGLE = True


class Color(Enum):
    RED = "RED"
    green = "green"


def load():
    options = dict(depth=2, flags=dict(a=1))
    weird = {"not-ident": 1}
    merged = dict(options, depth=5)
    combo = dict(mode=1) | dict(options, extra=2)
    banner = "🎉 launched"
    return options, weird, merged, combo, banner
