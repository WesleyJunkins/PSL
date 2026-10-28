"""psl/scale: out = in * gain + offset."""

from __future__ import annotations

from typing import Any

import numpy as np

import psl


def _number(value: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"expected a number, got {value!r}")
    return float(value)


class Scale(psl.Node):
    def on_init(self, params: dict[str, Any]) -> None:
        self.gain = _number(params.get("gain", 1.0))
        self.offset = _number(params.get("offset", 0.0))
        self.in_header: psl.SignalHeader | None = None
        self.open = False

    def on_header(self, port: psl.Port, header: psl.SignalHeader) -> None:
        self.in_header = header
        self._declare()

    def _declare(self) -> None:
        assert self.in_header is not None
        out = self.in_header.copy()
        if out.dtype in ("i16", "i32"):
            out.dtype = "f32"
        out.meta["scale"] = {"gain": self.gain, "offset": self.offset}
        self.declare_output("out", out)
        self.open = True

    def on_signal(self, port: psl.Port, chunk: psl.Chunk) -> None:
        if not self.open:
            self.log.warn("chunk before stream header; dropped", seq=chunk.seq)
            return
        y = chunk.data.astype(np.float64, copy=False) * self.gain + self.offset
        self.emit("out", chunk.replace(data=y))

    def on_end(self, port: psl.Port) -> None:
        if self.open:
            self.end("out")
            self.open = False

    def on_param(self, name: str, value: Any) -> bool:
        if name not in ("gain", "offset"):
            return False
        try:
            number = _number(value)
        except ValueError:
            return False
        setattr(self, name, number)
        if self.in_header is not None:
            self._declare()   # meta changed: re-announce
        return True


if __name__ == "__main__":
    psl.run(Scale)
