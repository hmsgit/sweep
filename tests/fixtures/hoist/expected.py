"""Hoisting fixture."""

from collections import Counter
import json
import os
import textwrap
import numpy
import requests

from mypkg.models import Thing
from mypkg.utils import helper


def handler(x):


    return json.dumps([Counter(x), numpy.array(x), helper(Thing(os.getpid()))])


def only_import():
    pass


def conditional():
    try:
        import orjson
    except ImportError:
        orjson = None
    return orjson


def dedup():
    return os.sep
