# sweep: ignore-file[docstring-start]
"""Scoped suppressions fixture."""

from enum import Enum


def wrong_shape():
    """Summary on the opening line.

    Kept as-is because of the file-level directive.
    """
    return None


class Legacy(Enum):  # sweep: ignore-block[casing-enum-key] wire format
    RED = 1
    GREEN = 2


# sweep: ignore-start[docstring-style] vendored helpers
def vendored_a():
    """Kept in Google style.

    Args:
        source: upstream project.
    """
    return None


def vendored_b():
    """Also kept.

    Args:
        source: upstream project.
    """
    return None
# sweep: ignore-end


def converted(x):
    """
    Outside the region, so converted.

    :param x: value.
    """
    return x


def pending(x: "Legacy") -> None:  # sweep: expect[string-annotations] until refactor
    return None


def stale() -> None:  # sweep: expect[imports-ban-local] refactored away
    return None
