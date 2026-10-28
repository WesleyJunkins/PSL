"""psl/moving-average: causal moving average, continuous across chunks."""

from __future__ import annotations

from typing import Any

import numpy as np

import psl


def _window(value: Any) -> int:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or int(value) != value:
        raise ValueError(f"window must be an integer, got {value!r}")
    w = int(value)
    if not 1 <= w <= 100000:
        raise ValueError(f"window must be between 1 and 100000, got {w}")
    return w


class MovingAverage(psl.Node):
    def on_init(self, params: dict[str, Any]) -> None:
        self.window = _window(params.get("window", 5))
        self.in_header: psl.SignalHeader | None = None
        self.tail: np.ndarray | None = None   # the last window-1 input samples, float64
        self.open = False

    def on_header(self, port: psl.Port, header: psl.SignalHeader) -> None:
        self.in_header = header
        self._reset()
        self._declare()

    def _reset(self) -> None:
        assert self.in_header is not None
        self.tail = np.zeros((self.window - 1, self.in_header.n_channels), dtype=np.float64)

    def _declare(self) -> None:
        assert self.in_header is not None
        out = self.in_header.copy()
        if out.dtype in ("i16", "i32"):
            out.dtype = "f32"   # an average of integers is not an integer
        out.meta["moving_average"] = {"window": self.window}
        self.declare_output("out", out)
        self.open = True

    def on_signal(self, port: psl.Port, chunk: psl.Chunk) -> None:
        if self.tail is None or self.in_header is None:
            self.log.warn("chunk before stream header; dropped", seq=chunk.seq)
            return
        x = chunk.data.astype(np.float64, copy=False)
        if x.shape[1] != self.tail.shape[1]:
            self.log.warn("channel count does not match the header; dropped", got=x.shape[1], expected=self.tail.shape[1])
            return
        w = self.window
        if w == 1:
            y = x
        else:
            buf = np.concatenate([self.tail, x], axis=0)
            cs = np.cumsum(buf, axis=0)
            cs = np.concatenate([np.zeros((1, buf.shape[1])), cs], axis=0)
            n = x.shape[0]
            y = (cs[w:w + n] - cs[0:n]) / w
            self.tail = buf[buf.shape[0] - (w - 1):].copy()
        self.emit("out", chunk.replace(data=y))

    def on_end(self, port: psl.Port) -> None:
        if self.open:
            self.end("out")
            self.open = False

    def on_param(self, name: str, value: Any) -> bool:
        if name != "window":
            return False
        try:
            w = _window(value)
        except ValueError:
            return False
        self.window = w
        if self.in_header is not None:
            self._reset()
            self._declare()   # the header's meta changed: re-announce before the next chunk
        return True


if __name__ == "__main__":
    psl.run(MovingAverage)
