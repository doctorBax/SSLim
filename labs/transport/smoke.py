"""Loopback protocol check with an ephemeral certificate and no stored secrets."""

from __future__ import annotations

import asyncio
import datetime as dt
import secrets
import ssl
import tempfile
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

from .client import connect
from .device import EchoPacketDevice
from .server import serve


_SAMPLE_IP_PACKET = bytes.fromhex("4500001400000000400100007f0000017f000001")


def _ephemeral_certificate(directory: Path) -> tuple[Path, Path]:
    key = ec.generate_private_key(ec.SECP256R1())
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")])
    now = dt.datetime.now(dt.timezone.utc)
    certificate = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - dt.timedelta(minutes=1))
        .not_valid_after(now + dt.timedelta(hours=1))
        .add_extension(x509.SubjectAlternativeName([x509.DNSName("localhost")]), False)
        .sign(key, hashes.SHA256())
    )
    certfile = directory / "cert.pem"
    keyfile = directory / "key.pem"
    certfile.write_bytes(certificate.public_bytes(serialization.Encoding.PEM))
    keyfile.write_bytes(
        key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )
    )
    return certfile, keyfile


async def _run(certfile: Path, keyfile: Path) -> None:
    secret = secrets.token_bytes(32)
    server = await serve(
        "127.0.0.1", 0, str(certfile), str(keyfile), secret, EchoPacketDevice
    )
    port = server.sockets[0].getsockname()[1]
    async with server:
        tunnel = await connect("127.0.0.1", port, "localhost", str(certfile), secret)
        try:
            await tunnel.send_packet(_SAMPLE_IP_PACKET)
            echoed = await asyncio.wait_for(tunnel.receive_packet(), timeout=3)
            if echoed != _SAMPLE_IP_PACKET:
                raise RuntimeError("packet changed in transit")

            packets = [_SAMPLE_IP_PACKET[:-1] + bytes([index]) for index in range(32)]
            await tunnel.send_batch(packets)
            for packet in packets:
                if await asyncio.wait_for(tunnel.receive_packet(), timeout=3) != packet:
                    raise RuntimeError("batch payload or packet order changed in transit")
        finally:
            await tunnel.close()
        try:
            unauthorized = await connect(
                "127.0.0.1", port, "localhost", str(certfile), secrets.token_bytes(32)
            )
        except ConnectionError:
            pass
        else:
            await unauthorized.close()
            raise RuntimeError("server accepted an invalid authentication secret")
        public_context = ssl.create_default_context(cafile=str(certfile))
        reader, writer = await asyncio.open_connection(
            "127.0.0.1", port, ssl=public_context, server_hostname="localhost"
        )
        try:
            writer.write(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            await writer.drain()
            response = await reader.read(1024)
            if not response.startswith(b"HTTP/1.1 200 OK\r\n"):
                raise RuntimeError("public endpoint failed")
        finally:
            writer.close()
            await writer.wait_closed()


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="vpn-bot-lab-") as path:
        certfile, keyfile = _ephemeral_certificate(Path(path))
        asyncio.run(_run(certfile, keyfile))
    print("loopback packet and batch, authentication, public endpoint: OK")


if __name__ == "__main__":
    main()
