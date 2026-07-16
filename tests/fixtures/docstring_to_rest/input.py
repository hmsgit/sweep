"""Module."""


def google_fn(a, b=2):
    """Add two numbers.

    Args:
        a (int): first operand.
        b (int): second operand, with a
            longer description.

    Returns:
        int: the sum.

    Raises:
        ValueError: on overflow.
    """
    return a + b


def numpy_fn(x):
    """Scale.

    Parameters
    ----------
    x : float
        The value.

    Returns
    -------
    float
        The scaled value.
    """
    return x * 2
