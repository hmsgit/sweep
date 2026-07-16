"""Annotations fixture."""

from typing import Annotated, Literal


def fetch(item: "Item", tag: Literal["x", "y"] = "x") -> "list[Item]":
    meta: Annotated["Item", "some metadata"] = None
    return [meta, item]


def keep(value: "Literal['a']") -> None:
    return None


class Item:
    parent: "Item | None" = None
