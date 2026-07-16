"""Suppression fixture."""


def cached():
    import json  # sweep: avoid-cycle heavy at import time
    return json


def annotated(x: "Thing") -> None:  # sweep: ignore[string-annotations] runtime introspection
    return None


def blanket_noqa():
    import os  # noqa
    return os


def blanket_type_ignore(x: "Thing") -> None:  # type: ignore
    return None


def coded_noqa_does_not_apply():
    import sys  # noqa: F401
    return sys


class Thing:
    pass
