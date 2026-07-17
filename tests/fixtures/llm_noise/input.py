"""LLM noise fixture."""


def send_email(recipient):
    """Send email."""
    # create the payload
    payload = create_payload(recipient)
    # deliver with retries because upstream flakes
    return deliver(payload)  # deliver the payload


def renamed(new_name, keep):
    """
    Do something meaningful here.

    :param old_name: description that went stale.
    :param keep: retained and documented.
    """
    return new_name, keep


def typed(x: int, y: dict[str, int]) -> bool:
    """
    Compare things carefully.

    :param x: left side.
    :type x: int
    :param y: right side, deliberately documented richer.
    :type y: mapping of str to int
    :returns: whether equal.
    :rtype: bool
    """
    return bool(x) and bool(y)
