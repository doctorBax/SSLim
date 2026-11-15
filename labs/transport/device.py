"""Packet device abstractions for a tunnel session."""

from __future__ import annotations

import asyncio
import ipaddress
from typing import Protocol

from .protocol import valid_ip_packet


class PacketDevice(Protocol):
    async def read_packet(self) -> bytes: ...

    async def write_packet(self, packet: bytes) -> None: ...

    async def close(self) -> None: ...


class EchoPacketDevice:
    """Loopback only: exercises the transport without forwarding traffic."""

    def __init__(self) -> None:
        self._queue: asyncio.Queue[bytes] = asyncio.Queue(maxsize=128)

    async def read_packet(self) -> bytes:
        return await self._queue.get()

    async def write_packet(self, packet: bytes) -> None:
        if not valid_ip_packet(packet):
            raise ValueError("invalid IP packet")
        await self._queue.put(packet)

    async def close(self) -> None:
        pass


def _checksum(data: bytes) -> int:
    if len(data) % 2:
        data += b"\x00"
    total = sum(int.from_bytes(data[i:i + 2], "big") for i in range(0, len(data), 2))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return (~total) & 0xFFFF


class IcmpResponderDevice:
    """Answer IPv4 ICMP echo on one virtual address; no Internet forwarding."""

    def __init__(self, address: str = "10.253.0.1") -> None:
        self._address = ipaddress.IPv4Address(address).packed
        self._queue: asyncio.Queue[bytes] = asyncio.Queue(maxsize=128)

    async def read_packet(self) -> bytes:
        return await self._queue.get()

    async def write_packet(self, packet: bytes) -> None:
        if not valid_ip_packet(packet) or packet[0] >> 4 != 4:
            return
        header_length = (packet[0] & 0x0F) * 4
        fragment_offset = int.from_bytes(packet[6:8], "big") & 0x3FFF
        if (
            packet[9] != 1
            or fragment_offset
            or packet[16:20] != self._address
            or len(packet) < header_length + 8
            or packet[header_length] != 8
            or packet[header_length + 1] != 0
        ):
            return
        reply = bytearray(packet)
        reply[12:16], reply[16:20] = packet[16:20], packet[12:16]
        reply[8] = 64
        reply[header_length] = 0
        reply[header_length + 2:header_length + 4] = b"\x00\x00"
        reply[header_length + 2:header_length + 4] = _checksum(
            bytes(reply[header_length:])
        ).to_bytes(2, "big")
        reply[10:12] = b"\x00\x00"
        reply[10:12] = _checksum(bytes(reply[:header_length])).to_bytes(2, "big")
        await self._queue.put(bytes(reply))

    async def close(self) -> None:
        pass
