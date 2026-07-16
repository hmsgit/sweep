"""House style fixture."""

from enum import Enum

TIMEOUT = 5
RETRIES: int = 3  # tuned ✓
TOGGLE = False
TOGGLE = True


class Color(Enum):
    RED = "RED"
    green = "green"


def load():
    options = {"depth": 2, "flags": {"a": 1}}
    weird = {"not-ident": 1}
    banner = "🎉 launched"
    return options, weird, banner
