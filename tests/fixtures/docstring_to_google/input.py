"""Module."""


def rest_fn(a, b):
    """Combine.

    :param a: first thing.
    :param str b: second thing, with a
        longer description.
    :type a: int
    :returns: combined result.
    :rtype: str
    :raises KeyError: if missing.
    """
    return str(a) + b
