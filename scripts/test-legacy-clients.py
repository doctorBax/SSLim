"""Check that current SSLim servers accept authentic historic TCP wire formats.

v0.1.0 and v0.2.0 sent "lab-packet/1"; v0.3.0+ sent "sync/1".
No TUN, external network or historical binaries are required.
"""
import asyncio
import hashlib
import hmac
import os
from pathlib import Path
import secrets
import socket
import ssl
import subprocess
import sys
import tempfile
import time


SAMPLE_IP = bytes(
    [0x45, 0, 0, 20, 0, 0, 0, 0, 64, 1, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1]
)


async def check_peer(port, certificate, secret, marker, expect_success, dynamic=False):
    context = ssl.create_default_context(cafile=str(certificate))
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.set_alpn_protocols(["http/1.1"])
    reader, writer = await asyncio.wait_for(
        asyncio.open_connection(
            "127.0.0.1", port, ssl=context, server_hostname="localhost"
        ),
        timeout=5,
    )
    try:
        stamp = str(int(time.time()))
        nonce = secrets.token_hex(16)
        material = ("vpn-bot-lab-transport-v1\0" + stamp + "\0" + nonce).encode("ascii")
        tag = hmac.new(secret, material, hashlib.sha256).hexdigest()
        address = "X-Session-Address: auto\r\n" if dynamic else ""
        request = (
            "POST /connect HTTP/1.1\r\nHost: localhost\r\n"
            f"Connection: Upgrade\r\nUpgrade: {marker}\r\n"
            f"X-Session-Time: {stamp}\r\nX-Session-Nonce: {nonce}\r\n"
            f"X-Session-Tag: {tag}\r\n{address}\r\n"
        ).encode("ascii")
        writer.write(request)
        await writer.drain()
        response = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
        if not expect_success:
            assert response.startswith(b"HTTP/1.1 404 "), response
            return
        assert response.startswith(b"HTTP/1.1 101 Switching Protocols\r\n"), response
        assert f"Upgrade: {marker}\r\n".encode("ascii") in response, response
        # Framing and echo must remain identical to 0.1/0.2 after the handshake.
        frame = bytes([1, 0, len(SAMPLE_IP)]) + SAMPLE_IP
        writer.write(frame)
        await writer.drain()
        assert await asyncio.wait_for(reader.readexactly(len(frame)), timeout=5) == frame
    finally:
        writer.close()
        try:
            await asyncio.wait_for(writer.wait_closed(), timeout=3)
        except (OSError, ssl.SSLError, TimeoutError):
            pass


async def run(binary):
    with tempfile.TemporaryDirectory(prefix="sslim-legacy-server-") as temporary:
        root = Path(temporary)
        certificate = root / "server.pem"
        private_key = root / "server.key"
        secret_path = root / "secret.bin"
        subprocess.run(
            [
                "openssl", "req", "-x509", "-newkey", "ec",
                "-pkeyopt", "ec_paramgen_curve:prime256v1",
                "-nodes", "-days", "1", "-subj", "/CN=localhost",
                "-addext", "subjectAltName=DNS:localhost",
                "-addext", "basicConstraints=critical,CA:FALSE",
                "-keyout", str(private_key), "-out", str(certificate),
            ],
            check=True, capture_output=True,
        )
        secret = os.urandom(32)
        secret_path.write_bytes(secret)
        # Reserve an ephemeral loopback port until the server is launched.
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        process = await asyncio.create_subprocess_exec(
            str(binary), "server", "--host", "127.0.0.1",
            "--port", str(port), "--device", "echo",
            "--cert", str(certificate), "--key", str(private_key),
            "--secret-file", str(secret_path),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            for _ in range(100):
                if process.returncode is not None:
                    raise RuntimeError("SSLim server exited before listening")
                try:
                    await check_peer(port, certificate, secret, "sync/1", True)
                    break
                except (OSError, ConnectionError):
                    await asyncio.sleep(0.05)
            else:
                raise TimeoutError("SSLim server did not start")
            # Exact historical markers verified against v0.1/v0.2 and v0.3+ tags.
            await check_peer(port, certificate, secret, "lab-packet/1", True)
            await check_peer(port, certificate, secret, "sync/1", True, dynamic=True)
            await check_peer(port, certificate, secret, "unknown/1", False)
            await check_peer(port, certificate, os.urandom(32), "lab-packet/1", False)
            await check_peer(port, certificate, os.urandom(32), "sync/1", False)
            print("Current server accepts v0.1/v0.2 and v0.3+ TCP wire formats: OK")
            print("Unknown marker and invalid legacy/current secrets: rejected")
        finally:
            process.terminate()
            try:
                await asyncio.wait_for(process.wait(), timeout=3)
            except TimeoutError:
                process.kill()
                await process.wait()


if __name__ == "__main__":
    asyncio.run(run(Path(sys.argv[1]).resolve()))
