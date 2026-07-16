"""Suppression fixture."""


def cached():
    import json  # sweep: avoid-cycle heavy at import time
    return json


def annotated(x: "Thing") -> None:  # sweep: ignore[string-annotations] runtime introspection
    return None


class Thing:
    pass
