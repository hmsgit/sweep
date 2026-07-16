"""Dict literal fixture."""


def build(rest):
    options = {"depth": 2, "mode": "a", **rest}
    keep = dict(rest)
    empty = dict()
    return options, keep, empty
