"""Hoisting fixture."""

import os
import requests

from mypkg.models import Thing


def handler(x):
    import json
    from collections import Counter

    import numpy
    from mypkg.utils import helper

    return json.dumps([Counter(x), numpy.array(x), helper(Thing(os.getpid()))])


def only_import():
    import textwrap


def conditional():
    try:
        import orjson
    except ImportError:
        orjson = None
    return orjson


def dedup():
    import os
    return os.sep
