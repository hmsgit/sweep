"""Module."""


def google_fn(a, b=2):
    """Add two numbers.

    :param a: first operand.
    :type a: int
    :param b: second operand, with a
        longer description.
    :type b: int
    :returns: the sum.
    :rtype: int
    :raises ValueError: on overflow.
    """
    return a + b


def numpy_fn(x):
    """Scale.

    :param x: The value.
    :type x: float
    :returns: The scaled value.
    :rtype: float
    """
    return x * 2
