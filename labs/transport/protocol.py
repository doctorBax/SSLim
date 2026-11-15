"""Version 1 packet framing for the laboratory transport."""

from __future__ import annotations

import asyncio
import struct
from enum import IntEnum


MAX_PACKET = 65_535
_HEADER = struct.Struct("!BH")


class FrameType(IntEnum):
    PACKET = 1
    PING = 2
    PONG = 3
    CLOSE = 4


class ProtocolError(ValueError):
    pass


def valid_ip_packet(packet: bytes) -> bool:
    """Reject malformed or padded IPv4/IPv6 packets at the tunnel boundary."""
    if not packet:
        return False
    version = packet[0] >> 4
    if version == 4:
        if len(packet) < 20:
            return False
        header_length = (packet[0] & 0x0F) * 4
        total_length = int.from_bytes(packet[2:4], "big")
        return 20 <= header_length <= len(packet) and total_length == len(packet)
    if version == 6:
        if len(packet) < 40:
            return False
        payload_length = int.from_bytes(packet[4:6], "big")
        return payload_length + 40 == len(packet)
    return False


def encode_frame(frame_type: FrameType, payload: bytes = b"") -> bytes:
    if len(payload) > MAX_PACKET:
        raise ProtocolError("frame exceeds maximum size")
    if frame_type == FrameType.PACKET and not valid_ip_packet(payload):
        raise ProtocolError("invalid IP packet")
    if frame_type != FrameType.PACKET and payload:
        raise ProtocolError("control frames cannot carry a payload")
    return _HEADER.pack(frame_type, len(payload)) + payload


async def read_frame(reader: asyncio.StreamReader) -> tuple[FrameType, bytes]:
    header = await reader.readexactly(_HEADER.size)
    raw_type, length = _HEADER.unpack(header)
    try:
        frame_type = FrameType(raw_type)
    except ValueError as exc:
        raise ProtocolError("unknown frame type") from exc
    payload = await reader.readexactly(length)
    if frame_type == FrameType.PACKET and not valid_ip_packet(payload):
        raise ProtocolError("invalid IP packet")
    if frame_type != FrameType.PACKET and payload:
        raise ProtocolError("control frame has a payload")
    return frame_type, payload
