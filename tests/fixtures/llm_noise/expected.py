"""LLM noise fixture."""


def send_email(recipient):
    payload = create_payload(recipient)
    # deliver with retries because upstream flakes
    return deliver(payload)


def renamed(new_name, keep):
    """
    Do something meaningful here.

    :param new_name:
    :param keep: retained and documented.
    """
    return new_name, keep


def typed(x: int, y: dict[str, int]) -> bool:
    """
    Compare things carefully.

    :param x: left side.
    :param y: right side, deliberately documented richer.
    :type y: mapping of str to int
    :returns: whether equal.
    """
    return bool(x) and bool(y)
