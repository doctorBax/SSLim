"""Linux TUN adapter; interface addresses and NAT are configured separately."""

from __future__ import annotations

import asyncio
import fcntl
import os
import struct

from .protocol import MAX_PACKET, valid_ip_packet


_TUNSETIFF = 0x400454CA
_IFF_TUN = 0x0001
_IFF_NO_PI = 0x1000
_BATCH_PACKETS = 32


class LinuxTun:
    def __init__(self, name: str) -> None:
        if not name or len(name.encode("ascii")) > 15:
            raise ValueError("TUN name must contain 1–15 ASCII bytes")
        self._loop = asyncio.get_running_loop()
        self._queue: asyncio.Queue[bytes] = asyncio.Queue(maxsize=512)
        self._reading = False
        self._fd = os.open("/dev/net/tun", os.O_RDWR | os.O_NONBLOCK)
        try:
            ifr = struct.pack("16sH", name.encode("ascii"), _IFF_TUN | _IFF_NO_PI)
            fcntl.ioctl(self._fd, _TUNSETIFF, ifr)
            self._loop.add_reader(self._fd, self._on_readable)
            self._reading = True
        except BaseException:
            os.close(self._fd)
            raise

    def _on_readable(self) -> None:
        for _ in range(_BATCH_PACKETS):
            if self._queue.full():
                self._loop.remove_reader(self._fd)
                self._reading = False
                return
            try:
                packet = os.read(self._fd, MAX_PACKET)
            except BlockingIOError:
                return
            if not packet:
                return
            if valid_ip_packet(packet):
                self._queue.put_nowait(packet)

    def _resume_reading(self) -> None:
        if self._fd >= 0 and not self._reading:
            self._loop.add_reader(self._fd, self._on_readable)
            self._reading = True

    async def read_packet(self) -> bytes:
        packet = await self._queue.get()
        self._resume_reading()
        return packet

    async def read_batch(self) -> list[bytes]:
        packets = [await self._queue.get()]
        while len(packets) < _BATCH_PACKETS and not self._queue.empty():
            packets.append(self._queue.get_nowait())
        self._resume_reading()
        return packets

    async def write_packet(self, packet: bytes) -> None:
        if not valid_ip_packet(packet):
            raise ValueError("invalid IP packet")
        while True:
            try:
                written = os.write(self._fd, packet)
                if written != len(packet):
                    raise OSError("partial TUN packet write")
                return
            except BlockingIOError:
                writable = self._loop.create_future()
                self._loop.add_writer(self._fd, writable.set_result, None)
                try:
                    await writable
                finally:
                    self._loop.remove_writer(self._fd)

    async def close(self) -> None:
        if self._fd >= 0:
            self._loop.remove_reader(self._fd)
            os.close(self._fd)
            self._fd = -1
