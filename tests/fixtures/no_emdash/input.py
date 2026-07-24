"""Fixture for no-emdash — dashes in prose become hyphens."""

# Set the flag — but only when the worker is ready.
ready = False  # applies to stages 1–5

MESSAGE = "wait — really?"


def f():
    """
    Do the thing — carefully.
    """
    return MESSAGE
