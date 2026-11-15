"""TLS server for the experimental packet transport."""

from __future__ import annotations

import asyncio
import logging
import ssl
from collections.abc import Callable

from .auth import ReplayGuard
from .device import PacketDevice
from .protocol import FrameType, ProtocolError, encode_frame, read_frame


_PUBLIC_PAGE = b"<!doctype html><title>Service</title><p>Service available.</p>"
_MAX_HEADERS = 8192
_LOG = logging.getLogger(__name__)


def tls_context(certfile: str, keyfile: str) -> ssl.SSLContext:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    context.load_cert_chain(certfile, keyfile)
    context.set_alpn_protocols(["http/1.1"])
    return context


def _http_response(status: str, body: bytes, *, head: bool = False) -> bytes:
    return (
        f"HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n"
        f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
    ).encode("ascii") + (b"" if head else body)


def _parse_request(raw: bytes) -> tuple[str, str, str, dict[str, str]]:
    if len(raw) > _MAX_HEADERS:
        raise ValueError("headers too large")
    lines = raw.decode("ascii").split("\r\n")
    method, path, version = lines[0].split(" ")
    if version not in ("HTTP/1.0", "HTTP/1.1"):
        raise ValueError("unsupported HTTP version")
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if not line:
            break
        name, separator, value = line.partition(":")
        if not separator or not name or name.lower() in headers:
            raise ValueError("invalid or duplicate header")
        headers[name.lower()] = value.strip()
    return method, path, version, headers


class TunnelServer:
    def __init__(self, secret: bytes, device_factory: Callable[[], PacketDevice]) -> None:
        if len(secret) < 32:
            raise ValueError("authentication secret must be at least 32 bytes")
        self._secret = secret
        self._device_factory = device_factory
        self._replay_guard = ReplayGuard()

    async def handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        device: PacketDevice | None = None
        try:
            raw = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
            try:
                method, path, version, headers = _parse_request(raw)
            except ValueError:
                writer.write(_http_response("400 Bad Request", b"Bad request"))
                await writer.drain()
                return
            ssl_object = writer.get_extra_info("ssl_object")
            if not (
                method == "POST"
                and path == "/connect"
                and version == "HTTP/1.1"
                and ssl_object is not None
                and ssl_object.version() == "TLSv1.3"
                and "upgrade" in {
                    token.strip().lower()
                    for token in headers.get("connection", "").split(",")
                }
                and headers.get("upgrade", "").lower() == "sync/1"
                and self._replay_guard.verify(
                    self._secret,
                    headers.get("x-session-time", ""),
                    headers.get("x-session-nonce", ""),
                    headers.get("x-session-tag", ""),
                )
            ):
                if method in ("GET", "HEAD") and path == "/":
                    status, body = "200 OK", _PUBLIC_PAGE
                else:
                    status, body = "404 Not Found", b"Not found"
                writer.write(_http_response(status, body, head=method == "HEAD"))
                await writer.drain()
                return
            device = self._device_factory()
            writer.write(
                b"HTTP/1.1 101 Switching Protocols\r\n"
                b"Connection: Upgrade\r\nUpgrade: sync/1\r\n\r\n"
            )
            await writer.drain()
            uplink = asyncio.create_task(self._uplink(reader, writer, device))
            downlink = asyncio.create_task(self._downlink(writer, device))
            done, pending = await asyncio.wait(
                (uplink, downlink), return_when=asyncio.FIRST_COMPLETED
            )
            for task in pending:
                task.cancel()
            await asyncio.gather(*pending, return_exceptions=True)
            for task in done:
                task.result()
        except (asyncio.IncompleteReadError, asyncio.LimitOverrunError, UnicodeError,
                ValueError, ProtocolError, TimeoutError, ConnectionError, OSError) as exc:
            _LOG.debug("session ended: %s", type(exc).__name__)
        finally:
            if device is not None:
                await device.close()
            writer.close()
            try:
                await writer.wait_closed()
            except (ConnectionError, TimeoutError):
                pass

    @staticmethod
    async def _uplink(
        reader: asyncio.StreamReader, writer: asyncio.StreamWriter, device: PacketDevice
    ) -> None:
        while True:
            frame_type, payload = await read_frame(reader)
            if frame_type == FrameType.PACKET:
                await device.write_packet(payload)
            elif frame_type == FrameType.PING:
                writer.write(encode_frame(FrameType.PONG))
                await writer.drain()
            elif frame_type == FrameType.CLOSE:
                return

    @staticmethod
    async def _downlink(writer: asyncio.StreamWriter, device: PacketDevice) -> None:
        read_batch = getattr(device, "read_batch", None)
        while True:
            if read_batch is None:
                writer.write(encode_frame(FrameType.PACKET, await device.read_packet()))
            else:
                packets = await read_batch()
                writer.write(b"".join(encode_frame(FrameType.PACKET, packet) for packet in packets))
            await writer.drain()


async def serve(
    host: str,
    port: int,
    certfile: str,
    keyfile: str,
    secret: bytes,
    device_factory: Callable[[], PacketDevice],
) -> asyncio.AbstractServer:
    endpoint = TunnelServer(secret, device_factory)
    return await asyncio.start_server(
        endpoint.handle,
        host,
        port,
        ssl=tls_context(certfile, keyfile),
        ssl_handshake_timeout=5,
        ssl_shutdown_timeout=2,
        limit=_MAX_HEADERS,
    )
