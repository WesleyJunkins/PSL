"""psl/synthetic: an input driver that generates an ECG-like, sine or noise signal."""

from __future__ import annotations

import math
from typing import Any

import numpy as np

import psl

NS_PER_S = 1_000_000_000

#: ECG beat as a sum of Gaussians over one beat of phase in [0, 1):
#: (centre, width, amplitude) for the P wave, Q, R, S and T wave.
ECG_WAVES: tuple[tuple[float, float, float], ...] = (
    (0.18, 0.030, 0.15),
    (0.37, 0.010, -0.10),
    (0.40, 0.012, 1.00),
    (0.43, 0.010, -0.25),
    (0.62, 0.045, 0.30),
)


def ecg_shape(phase: np.ndarray) -> np.ndarray:
    """Unit-amplitude ECG beat sampled at ``phase`` (beats, any real number)."""
    y = np.zeros_like(phase, dtype=np.float64)
    for centre, width, amp in ECG_WAVES:
        d = phase - centre
        d -= np.round(d)   # wrap to [-0.5, 0.5) so the beat is periodic
        y += amp * np.exp(-0.5 * (d / width) ** 2)
    return y


def _number(value: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"expected a number, got {value!r}")
    return float(value)


class Synthetic(psl.Node):
    def on_init(self, params: dict[str, Any]) -> None:
        self.waveform = str(params.get("waveform", "ecg"))
        if self.waveform not in ("ecg", "sine", "noise"):
            raise ValueError(f"unknown waveform {self.waveform!r}")
        self.srate = _number(params.get("srate", 250))
        if self.srate <= 0:
            raise ValueError("srate must be > 0")
        self.labels = [str(c) for c in params.get("channels", ["ECG"])]
        if not self.labels:
            raise ValueError("channels must not be empty")
        self.chunk_ms = int(params.get("chunk_ms", 20))
        if self.chunk_ms < 1:
            raise ValueError("chunk_ms must be >= 1")
        self.amplitude = _number(params.get("amplitude", 1.0))
        self.frequency = _number(params.get("frequency", 1.0))
        self.bpm = _number(params.get("bpm", 60))
        if self.bpm <= 0:
            raise ValueError("bpm must be > 0")
        self.seed = int(params.get("seed", 0))
        self.rng = np.random.default_rng(self.seed)

        if self.waveform == "ecg":
            unit, modality = "mV", "ecg"
        else:
            unit, modality = "au", "other"
        self.header = psl.SignalHeader(
            srate=self.srate,
            channels=[psl.Channel(label=label, unit=unit, modality=modality) for label in self.labels],
            dtype="f32",
            source={"device": "psl/synthetic", "waveform": self.waveform, "timing": "t0 = now() - chunk duration"},
        )
        self.n = 0            # samples emitted so far; keeps the waveform continuous across chunks
        self.ticks = 0        # timer ticks so far
        self.phase = 0.0      # beats (ecg) or cycles (sine) at sample n

    def on_start(self) -> None:
        self.declare_output("out", self.header)
        self.every(self.chunk_ms / 1000.0, self.tick)

    def tick(self) -> None:
        # Samples per tick are srate * chunk_ms / 1000; a fractional remainder is
        # carried over, so the average rate is exactly srate (e.g. 1.5 -> 1, 2, 1, 2).
        self.ticks += 1
        target = math.floor(self.ticks * self.srate * self.chunk_ms / 1000.0 + 1e-9)
        count = target - self.n
        if count <= 0:
            return
        data = self.generate(count)
        self.n = target
        t0 = self.now() - round(count * NS_PER_S / self.srate)
        self.emit("out", psl.Chunk(t0=t0, data=data))

    def generate(self, count: int) -> np.ndarray:
        """``count`` samples for every channel as float32 ``[count, n_channels]``."""
        n_ch = len(self.labels)
        if self.waveform == "noise":
            y = self.amplitude * self.rng.standard_normal((count, n_ch))
        elif self.waveform == "sine":
            step = self.frequency / self.srate
            cycles = self.phase + np.arange(count, dtype=np.float64) * step
            self.phase = (self.phase + count * step) % 1.0
            shift = 0.25 * np.arange(n_ch, dtype=np.float64)   # quarter cycle per channel
            y = self.amplitude * np.sin(2.0 * np.pi * (cycles[:, None] + shift[None, :]))
        else:
            step = self.bpm / 60.0 / self.srate
            beats = self.phase + np.arange(count, dtype=np.float64) * step
            self.phase = (self.phase + count * step) % 1.0
            y = np.repeat((self.amplitude * ecg_shape(beats))[:, None], n_ch, axis=1)
        return np.ascontiguousarray(y, dtype=np.float32)

    def on_param(self, name: str, value: Any) -> bool:
        if name == "amplitude":
            try:
                self.amplitude = _number(value)
            except ValueError:
                return False
            return True
        if name == "bpm":
            try:
                bpm = _number(value)
            except ValueError:
                return False
            if bpm <= 0:
                return False
            self.bpm = bpm
            return True
        return False


if __name__ == "__main__":
    psl.run(Synthetic)
