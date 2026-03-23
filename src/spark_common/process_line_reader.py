from __future__ import annotations

import threading
from queue import Empty, Queue
from typing import Optional, TextIO


class ProcessLineReader:
    """Drain a text stream on a background thread and expose line reads with timeouts."""

    def __init__(self, stream: TextIO) -> None:
        self._stream = stream
        self._lines: Queue[Optional[str]] = Queue()
        self._thread = threading.Thread(target=self._drain, daemon=True)
        self._thread.start()

    def _drain(self) -> None:
        try:
            while True:
                line = self._stream.readline()
                if not line:
                    break
                self._lines.put(line.rstrip("\n"))
        finally:
            self._lines.put(None)

    def read_line(self, wait: float) -> Optional[str]:
        try:
            line = self._lines.get(timeout=max(wait, 0))
        except Empty:
            return None
        if line is None:
            self._lines.put(None)
            return None
        return line

    def join(self, timeout: float | None = None) -> None:
        self._thread.join(timeout=timeout)
