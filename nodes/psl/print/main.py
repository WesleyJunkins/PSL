"""psl/print: an output driver that logs what it receives."""

from __future__ import annotations

import json
from typing import Any

import numpy as np

import psl

MAX_FIRST = 8   # values of the first sample shown per line


def _fmt_values(row: np.ndarray) -> str:
    values = [f"{float(v):.5g}" for v in row[:MAX_FIRST]]
    if row.shape[0] > MAX_FIRST:
        values.append("...")
    return "[" + ", ".join(values) + "]"


class PrintNode(psl.Node):
    def on_init(self, params: dict[str, Any]) -> None:
        every = params.get("every", 25)
        if isinstance(every, bool) or not isinstance(every, (int, float)) or int(every) != every or int(every) < 1:
            raise ValueError(f"every must be an integer >= 1, got {every!r}")
        self.every_n = int(every)
        self.counts: dict[tuple[str, int], int] = {}

    def on_header(self, port: psl.Port, header: psl.SignalHeader | psl.EventHeader) -> None:
        if isinstance(header, psl.SignalHeader):
            labels = ", ".join(f"{c.label} ({c.unit}, {c.modality})" for c in header.channels)
            self.log.info(
                f"{port.name} header: signal srate={header.srate:g} dtype={header.dtype} "
                f"channels=[{labels}] source={json.dumps(header.source, default=str)} meta={json.dumps(header.meta, default=str)}",
                source=port.source,
            )
        else:
            self.log.info(
                f"{port.name} header: event labels={header.labels} source={json.dumps(header.source, default=str)}",
                source=port.source,
            )
        self.counts[(port.name, port.source_index)] = 0

    def on_signal(self, port: psl.Port, chunk: psl.Chunk) -> None:
        key = (port.name, port.source_index)
        count = self.counts.get(key, 0)
        self.counts[key] = count + 1
        if count % self.every_n != 0:
            return
        first = _fmt_values(chunk.data[0]) if chunk.n_samples else "[]"
        self.log.info(
            f"{port.name} seq={chunk.seq} t0={chunk.t0} shape={chunk.data.shape} first={first}",
            source=port.source, seq=chunk.seq, t0=chunk.t0, n_samples=chunk.n_samples, n_channels=chunk.n_channels,
        )

    def on_event(self, port: psl.Port, event: psl.Event) -> None:
        self.log.info(
            f"{port.name} seq={event.seq} t={event.t} label={event.label} data={json.dumps(event.data, default=str)}",
            source=port.source, seq=event.seq, t=event.t, label=event.label,
        )

    def on_end(self, port: psl.Port) -> None:
        self.log.info(f"{port.name} end of stream", source=port.source)


if __name__ == "__main__":
    psl.run(PrintNode)
