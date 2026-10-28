"""Socket transport: connect to PSL_CONTROL / PSL_DATA and move framed bytes.

An address starting with ``/`` is a Unix domain socket path; anything else is
``host:port`` TCP (the Windows form, and the future remote form). The bytes are
identical over both.
"""

from __future__ import annotations

import socket
import threading
from typing import Any

from ._protocol import (
    CONTROL_LEN_SIZE,
    HEADER_SIZE,
    FrameHeader,
    decode_control,
    encode_control,
    unpack_control_length,
)


class ConnectionClosed(EOFError):
    """The peer closed the connection (clean EOF while a read was pending)."""


def parse_address(address: str) -> tuple[str, str | tuple[str, int]]:
    """``("unix", path)`` or ``("tcp", (host, port))``."""
    address = address.strip()
    if not address:
        raise ValueError("empty socket address")
    if address.startswith("/"):
        return "unix", address
    host, sep, port = address.rpartition(":")
    if not sep or not port.isdigit():
        raise ValueError(f"TCP address must be host:port, got {address!r}")
    host = host.strip("[]") or "127.0.0.1"
    return "tcp", (host, int(port))


def connect(address: str, timeout: float | None = 10.0) -> socket.socket:
    """Open a blocking stream socket to ``address``.

    ``timeout`` bounds the connect only; the returned socket is blocking.
    """
    kind, target = parse_address(address)
    if kind == "unix":
        family = getattr(socket, "AF_UNIX", None)
        if family is None:
            raise OSError(f"Unix domain sockets are not available on this platform ({address})")
        sock = socket.socket(family, socket.SOCK_STREAM)
        try:
            sock.settimeout(timeout)
            sock.connect(target)
        except BaseException:
            sock.close()
            raise
    else:
        assert isinstance(target, tuple)
        sock = socket.create_connection(target, timeout=timeout)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    sock.settimeout(None)
    return sock


class Connection:
    """A framed, thread-safe-for-writes wrapper around one stream socket.

    Reads must come from one thread at a time (the SDK uses one reader thread
    per socket). Writes may come from any thread: every write holds the send
    lock, so frames never interleave.
    """

    def __init__(self, sock: socket.socket) -> None:
        self._sock = sock
        self._send_lock = threading.Lock()
        self._closed = False

    @property
    def closed(self) -> bool:
        return self._closed

    @property
    def socket(self) -> socket.socket:
        return self._sock

    # -- raw ---------------------------------------------------------------

    def read_exact(self, n: int) -> bytes:
        """Read exactly ``n`` bytes or raise ``ConnectionClosed``."""
        if n < 0:
            raise ValueError("negative read length")
        if n == 0:
            return b""
        buf = bytearray(n)
        view = memoryview(buf)
        got = 0
        while got < n:
            k = self._sock.recv_into(view[got:], n - got)
            if k == 0:
                raise ConnectionClosed(f"connection closed after {got} of {n} bytes")
            got += k
        return bytes(buf)

    def write_all(self, data: bytes | bytearray | memoryview) -> None:
        """Send every byte of ``data``, atomically with respect to other writers."""
        with self._send_lock:
            self._sock.sendall(data)

    # -- control plane -----------------------------------------------------

    def read_control(self) -> dict[str, Any]:
        n = unpack_control_length(self.read_exact(CONTROL_LEN_SIZE))
        return decode_control(self.read_exact(n))

    def write_control(self, msg: dict[str, Any]) -> None:
        self.write_all(encode_control(msg))

    # -- data plane --------------------------------------------------------

    def read_frame(self) -> tuple[FrameHeader, bytes]:
        header = FrameHeader.unpack(self.read_exact(HEADER_SIZE))
        header.validate()
        payload = self.read_exact(header.payload_length)
        return header, payload

    def write_frame(self, frame: bytes) -> None:
        self.write_all(frame)

    # -- lifecycle ---------------------------------------------------------

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            # shutdown() wakes a reader blocked in recv() on every platform;
            # close() alone does not on macOS.
            self._sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            self._sock.close()
        except OSError:
            pass
