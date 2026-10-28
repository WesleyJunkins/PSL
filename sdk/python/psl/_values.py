"""Value types: channels, stream headers, chunks, events and ports.

Plain data classes shared by the protocol codec (``_protocol``), the node
runtime (``_node``) and the offline test harness (``testing``). They carry no
transport state. See docs/02-data-model.md for the semantics.
"""

from __future__ import annotations

import copy
import dataclasses
from dataclasses import dataclass, field
from typing import Any, ClassVar

import numpy as np

#: Sample types a signal stream may carry, by wire name (docs/02 §Stream header).
DTYPES: tuple[str, ...] = ("f32", "f64", "i16", "i32")

#: numpy dtype for each wire name. Explicitly little-endian: PSL never uses
#: big-endian, so ``'<'`` is spelled out rather than relying on the host order.
NUMPY_DTYPES: dict[str, np.dtype[Any]] = {
    "f32": np.dtype("<f4"),
    "f64": np.dtype("<f8"),
    "i16": np.dtype("<i2"),
    "i32": np.dtype("<i4"),
}

#: Modality vocabulary from docs/02. Documentation only; the runtime never
#: validates it.
MODALITIES: tuple[str, ...] = (
    "eeg", "ecg", "emg", "eog", "ppg", "eda", "resp", "temp", "acc", "gyro",
    "mag", "bp", "spo2", "marker", "other",
)

NS_PER_S: int = 1_000_000_000


def _dtype_name_of(array: np.ndarray) -> str | None:
    """Wire name for an array's dtype, or None if it has no wire equivalent."""
    for name, npdt in NUMPY_DTYPES.items():
        if array.dtype == npdt:
            return name
    return None


@dataclass
class Channel:
    """One channel of a signal stream."""

    label: str
    unit: str = "au"
    modality: str = "other"

    def to_json(self) -> dict[str, str]:
        return {"label": self.label, "unit": self.unit, "modality": self.modality}

    @classmethod
    def from_json(cls, obj: Any) -> Channel:
        if isinstance(obj, Channel):
            return dataclasses.replace(obj)
        if isinstance(obj, str):
            return cls(label=obj)
        if not isinstance(obj, dict) or "label" not in obj:
            raise ValueError(f"channel entry must be an object with a label, got {obj!r}")
        return cls(
            label=str(obj["label"]),
            unit=str(obj.get("unit", "au")),
            modality=str(obj.get("modality", "other")),
        )


@dataclass
class SignalHeader:
    """Header of a ``signal`` stream (docs/02 §Stream header)."""

    srate: float
    channels: list[Channel]
    dtype: str = "f32"
    source: dict[str, Any] = field(default_factory=dict)
    meta: dict[str, Any] = field(default_factory=dict)

    #: Port type this header describes; matches the JSON ``type`` field.
    type: ClassVar[str] = "signal"

    def __post_init__(self) -> None:
        self.srate = float(self.srate)
        if not self.srate > 0:
            raise ValueError(f"srate must be > 0, got {self.srate}")
        if self.dtype not in NUMPY_DTYPES:
            raise ValueError(f"unknown dtype {self.dtype!r}; expected one of {DTYPES}")
        self.channels = [Channel.from_json(c) for c in self.channels]
        self.source = dict(self.source)
        self.meta = dict(self.meta)

    @property
    def n_channels(self) -> int:
        return len(self.channels)

    @property
    def labels(self) -> list[str]:
        return [c.label for c in self.channels]

    def copy(self) -> SignalHeader:
        """Deep copy, so a module can append to ``meta`` without touching the input."""
        return SignalHeader(
            srate=self.srate,
            channels=[dataclasses.replace(c) for c in self.channels],
            dtype=self.dtype,
            source=copy.deepcopy(self.source),
            meta=copy.deepcopy(self.meta),
        )

    def to_json(self) -> dict[str, Any]:
        return {
            "type": "signal",
            "srate": self.srate,
            "dtype": self.dtype,
            "channels": [c.to_json() for c in self.channels],
            "source": copy.deepcopy(self.source),
            "meta": copy.deepcopy(self.meta),
        }

    @classmethod
    def from_json(cls, obj: dict[str, Any]) -> SignalHeader:
        if not isinstance(obj, dict):
            raise ValueError("stream header must be a JSON object")
        if obj.get("type", "signal") != "signal":
            raise ValueError(f"expected a signal header, got type {obj.get('type')!r}")
        for key in ("srate", "channels"):
            if key not in obj:
                raise ValueError(f"signal header is missing required field {key!r}")
        return cls(
            srate=obj["srate"],
            channels=list(obj["channels"]),
            dtype=str(obj.get("dtype", "f32")),
            source=dict(obj.get("source") or {}),
            meta=dict(obj.get("meta") or {}),
        )


@dataclass
class EventHeader:
    """Header of an ``event`` stream (docs/02 §Stream header)."""

    labels: list[str] = field(default_factory=list)
    schema: dict[str, Any] = field(default_factory=dict)
    source: dict[str, Any] = field(default_factory=dict)
    meta: dict[str, Any] = field(default_factory=dict)

    type: ClassVar[str] = "event"

    def __post_init__(self) -> None:
        self.labels = [str(label) for label in self.labels]
        self.schema = dict(self.schema)
        self.source = dict(self.source)
        self.meta = dict(self.meta)

    def copy(self) -> EventHeader:
        return EventHeader(
            labels=list(self.labels),
            schema=copy.deepcopy(self.schema),
            source=copy.deepcopy(self.source),
            meta=copy.deepcopy(self.meta),
        )

    def to_json(self) -> dict[str, Any]:
        return {
            "type": "event",
            "labels": list(self.labels),
            "schema": copy.deepcopy(self.schema),
            "source": copy.deepcopy(self.source),
            "meta": copy.deepcopy(self.meta),
        }

    @classmethod
    def from_json(cls, obj: dict[str, Any]) -> EventHeader:
        if not isinstance(obj, dict):
            raise ValueError("stream header must be a JSON object")
        if obj.get("type", "event") != "event":
            raise ValueError(f"expected an event header, got type {obj.get('type')!r}")
        return cls(
            labels=list(obj.get("labels") or []),
            schema=dict(obj.get("schema") or {}),
            source=dict(obj.get("source") or {}),
            meta=dict(obj.get("meta") or {}),
        )


#: Either kind of stream header.
Header = SignalHeader | EventHeader


@dataclass(eq=False)
class Chunk:
    """A contiguous block of samples from one signal stream (docs/02 §Signal chunk).

    ``data`` is always a 2-D array shaped ``[n_samples, n_channels]``; a 1-D
    array is treated as one channel. ``srate`` is not on the wire: the SDK fills
    it from the stream header so that ``time_of`` works without per-sample
    timestamps (see sdk/python/NOTES.md).
    """

    t0: int
    data: np.ndarray
    timestamps: np.ndarray | None = None
    t_device: int = 0
    seq: int = 0
    srate: float | None = None

    def __post_init__(self) -> None:
        data = np.asarray(self.data)
        if data.ndim == 0:
            data = data.reshape(1, 1)
        elif data.ndim == 1:
            data = data.reshape(-1, 1)
        if data.ndim != 2:
            raise ValueError(f"chunk data must be [n_samples, n_channels], got shape {data.shape}")
        self.data = data
        self.t0 = int(self.t0)
        self.t_device = int(self.t_device)
        self.seq = int(self.seq)
        if self.timestamps is not None:
            ts = np.asarray(self.timestamps, dtype=np.int64).reshape(-1)
            if ts.shape[0] != data.shape[0]:
                raise ValueError(
                    f"timestamps has {ts.shape[0]} entries but the chunk has {data.shape[0]} samples"
                )
            self.timestamps = ts
        if self.srate is not None:
            self.srate = float(self.srate)

    @property
    def n_samples(self) -> int:
        return int(self.data.shape[0])

    @property
    def n_channels(self) -> int:
        return int(self.data.shape[1])

    @property
    def duration_ns(self) -> int:
        """Nominal duration ``n_samples / srate`` in ns; 0 if the rate is unknown."""
        if self.srate is None:
            return 0
        return round(self.n_samples * NS_PER_S / self.srate)

    def replace(self, **fields: Any) -> Chunk:
        """A copy with some fields replaced, e.g. ``chunk.replace(data=y)``."""
        return dataclasses.replace(self, **fields)

    def time_of(self, i: int) -> int:
        """Session time of sample ``i``: the per-sample timestamp if present, else ``t0 + i / srate``."""
        n = self.n_samples
        if i < 0:
            i += n
        if not 0 <= i < n:
            raise IndexError(f"sample index {i} out of range for {n} samples")
        if self.timestamps is not None:
            return int(self.timestamps[i])
        if self.srate is None:
            raise ValueError("chunk has neither per-sample timestamps nor a sample rate; cannot compute time_of")
        return self.t0 + round(i * NS_PER_S / self.srate)

    def times(self) -> np.ndarray:
        """Session time of every sample as an int64 array (vectorised ``time_of``)."""
        if self.timestamps is not None:
            return self.timestamps.copy()
        if self.srate is None:
            raise ValueError("chunk has neither per-sample timestamps nor a sample rate; cannot compute times")
        idx = np.arange(self.n_samples, dtype=np.float64)
        return self.t0 + np.rint(idx * (NS_PER_S / self.srate)).astype(np.int64)


@dataclass
class Event:
    """One timestamped record on an event stream (docs/02 §Event)."""

    t: int
    label: str
    data: Any = None
    t_device: int = 0
    seq: int = 0

    def __post_init__(self) -> None:
        self.t = int(self.t)
        self.label = str(self.label)
        self.t_device = int(self.t_device)
        self.seq = int(self.seq)

    def replace(self, **fields: Any) -> Event:
        return dataclasses.replace(self, **fields)


@dataclass(frozen=True)
class Port:
    """The port a callback refers to.

    For input ports ``source`` is the stream id of the frame's origin (e.g.
    ``"ecg.out"``) and ``source_index`` its index in the port's source list from
    ``config``. For output ports ``source`` is the port's own stream id.
    """

    name: str
    index: int
    type: str
    source: str = ""
    source_index: int = 0
