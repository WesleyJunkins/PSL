"""psl: the Python SDK for PSL nodes.

Subclass ``psl.Node``, override the ``on_*`` callbacks you need, and finish
your entry script with ``psl.run(YourNode)``. See docs/05-node-sdk.md and
sdk/python/README.md.
"""


from __future__ import annotations

import sys as _sys

if _sys.version_info < (3, 11):  # the SDK uses 3.10+ syntax; fail with a clear message, not a SyntaxError
    raise SystemExit(f"psl: Python 3.11 or newer is required, this interpreter is {_sys.version.split()[0]}")


from ._node import Logger, Node, NodeFailed, Timer, run
from ._values import (
    DTYPES,
    MODALITIES,
    NUMPY_DTYPES,
    Channel,
    Chunk,
    Event,
    EventHeader,
    Port,
    SignalHeader,
)

__version__ = "0.1.0"

__all__ = [
    "Node",
    "run",
    "Channel",
    "SignalHeader",
    "EventHeader",
    "Chunk",
    "Event",
    "Port",
    "Timer",
    "Logger",
    "NodeFailed",
    "DTYPES",
    "MODALITIES",
    "NUMPY_DTYPES",
    "__version__",
]
