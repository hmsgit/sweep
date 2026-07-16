"""Module."""


def rest_fn(a, b):
    """Combine.

    Args:
        a (int): first thing.
        b (str): second thing, with a
            longer description.

    Returns:
        str: combined result.

    Raises:
        KeyError: if missing.
    """
    return str(a) + b
