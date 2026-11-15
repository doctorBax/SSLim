"""TLS client for the experimental packet transport."""

from __future__ import annotations

import asyncio
import ssl
import time

from .auth import make_tag, new_nonce
from .protocol import FrameType, encode_frame, read_frame


class Tunnel:
    def __init__(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        self._reader = reader
        self._writer = writer
        self._write_lock = asyncio.Lock()

    async def send_packet(self, packet: bytes) -> None:
        async with self._write_lock:
            self._writer.write(encode_frame(FrameType.PACKET, packet))
            await self._writer.drain()

    async def send_batch(self, packets: list[bytes]) -> None:
        payload = b"".join(encode_frame(FrameType.PACKET, packet) for packet in packets)
        async with self._write_lock:
            self._writer.write(payload)
            await self._writer.drain()

    async def receive_packet(self) -> bytes:
        while True:
            frame_type, payload = await read_frame(self._reader)
            if frame_type == FrameType.PACKET:
                return payload
            if frame_type == FrameType.PING:
                async with self._write_lock:
                    self._writer.write(encode_frame(FrameType.PONG))
                    await self._writer.drain()
            if frame_type == FrameType.CLOSE:
                raise ConnectionError("remote endpoint closed the tunnel")

    async def ping(self) -> None:
        async with self._write_lock:
            self._writer.write(encode_frame(FrameType.PING))
            await self._writer.drain()

    async def close(self) -> None:
        if not self._writer.is_closing():
            try:
                self._writer.write(encode_frame(FrameType.CLOSE))
                await asyncio.wait_for(self._writer.drain(), timeout=2)
            except (ConnectionError, TimeoutError):
                pass
        self._writer.close()
        try:
            await asyncio.wait_for(self._writer.wait_closed(), timeout=2)
        except (ssl.SSLError, ConnectionError, TimeoutError):
            pass


async def connect(
    host: str,
    port: int,
    server_name: str,
    ca_file: str | None,
    secret: bytes,
) -> Tunnel:
    if not server_name or any(c in server_name for c in "\r\n "):
        raise ValueError("invalid server name")
    context = ssl.create_default_context(cafile=ca_file)
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    context.set_alpn_protocols(["http/1.1"])
    reader, writer = await asyncio.wait_for(
        asyncio.open_connection(
            host, port, ssl=context, server_hostname=server_name, limit=8192
        ),
        timeout=10,
    )
    try:
        timestamp = int(time.time())
        nonce = new_nonce()
        tag = make_tag(secret, timestamp, nonce)
        request = (
            f"POST /connect HTTP/1.1\r\nHost: {server_name}\r\n"
            "Connection: Upgrade\r\nUpgrade: sync/1\r\n"
            f"X-Session-Time: {timestamp}\r\n"
            f"X-Session-Nonce: {nonce}\r\nX-Session-Tag: {tag}\r\n\r\n"
        ).encode("ascii")
        writer.write(request)
        await writer.drain()
        response = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
        if not response.startswith(b"HTTP/1.1 101 Switching Protocols\r\n"):
            raise ConnectionError("server rejected the session")
        return Tunnel(reader, writer)
    except BaseException:
        writer.close()
        await writer.wait_closed()
        raise
