"""psl/csv-writer: an output driver that writes one CSV row per sample."""

from __future__ import annotations

import csv
import os
from typing import Any, TextIO

import psl


class CsvWriter(psl.Node):
    def on_init(self, params: dict[str, Any]) -> None:
        path = params.get("path")
        if not isinstance(path, str) or not path:
            raise ValueError("param 'path' is required and must be a non-empty string")
        self.path = path
        self.append = bool(params.get("append", False))
        parent = os.path.dirname(os.path.abspath(path))
        os.makedirs(parent, exist_ok=True)
        resume = self.append and os.path.exists(path) and os.path.getsize(path) > 0
        self.f: TextIO = open(path, "a" if self.append else "w", newline="", encoding="utf-8")
        self.w = csv.writer(self.f)
        self.skip_next_header = resume   # appending to existing data: do not repeat the header row
        self.labels: list[str] | None = None
        self.rows = 0
        self.log.info("writing", path=path, append=self.append)

    def on_header(self, port: psl.Port, header: psl.SignalHeader) -> None:
        labels = [c.label for c in header.channels]
        if self.skip_next_header:
            self.skip_next_header = False
            self.labels = labels
            return
        if labels != self.labels:   # a re-announced identical header does not repeat the row
            self.w.writerow(["t_ns", *labels])
            self.labels = labels

    def on_signal(self, port: psl.Port, chunk: psl.Chunk) -> None:
        data = chunk.data
        for i in range(chunk.n_samples):
            self.w.writerow([chunk.time_of(i), *(str(v) for v in data[i])])
        self.rows += chunk.n_samples

    def on_end(self, port: psl.Port) -> None:
        self.f.flush()

    def on_stop(self) -> None:
        self.f.flush()
        self.f.close()
        self.log.info("closed", path=self.path, rows=self.rows)


if __name__ == "__main__":
    psl.run(CsvWriter)
