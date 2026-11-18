"""Regression: SSLim's current TCP client must connect to a v0.2.0-style server.

Runs entirely on loopback, with an ephemeral TLS certificate and secret.
No Windows adapter or external network is required.
"""
import asyncio
import hashlib
import hmac
import os
from pathlib import Path
import ssl
import subprocess
import sys
import tempfile
import time


async def legacy_peer(reader, writer, secret, attempts):
    try:
        request = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
        lines = request.decode("ascii").split("\r\n")
        headers = dict(
            (key.lower(), value.strip())
            for line in lines[1:]
            if ": " in line
            for key, value in [line.split(":", 1)]
        )
        marker = headers.get("upgrade", "")
        attempts.append(marker)
        stamp = headers.get("x-session-time", "")
        nonce = headers.get("x-session-nonce", "")
        proof = headers.get("x-session-tag", "")
        expected = hmac.new(
            secret,
            ("vpn-bot-lab-transport-v1\0" + stamp + "\0" + nonce).encode("ascii"),
            hashlib.sha256,
        ).hexdigest()
        if (
            marker != "lab-packet/1"
            or not hmac.compare_digest(expected, proof)
            or abs(int(stamp) - int(time.time())) > 120
        ):
            writer.write(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            await writer.drain()
            return
        writer.write(
            b"HTTP/1.1 101 Switching Protocols\r\n"
            b"Connection: Upgrade\r\nUpgrade: lab-packet/1\r\n\r\n"
        )
        await writer.drain()
        header = await asyncio.wait_for(reader.readexactly(3), timeout=5)
        assert header[0] == 1, header
        size = int.from_bytes(header[1:], "big")
        payload = await asyncio.wait_for(reader.readexactly(size), timeout=5)
        writer.write(header + payload)
        await writer.drain()
    except (asyncio.IncompleteReadError, ConnectionError, TimeoutError):
        pass
    finally:
        writer.close()
        try:
            await writer.wait_closed()
        except (OSError, ssl.SSLError):
            pass


async def test(binary):
    with tempfile.TemporaryDirectory(prefix="sslim-legacy-") as temporary:
        root = Path(temporary)
        certificate, private_key, secret_path = [
            root / name for name in ("cert.pem", "key.pem", "secret.bin")
        ]
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
        attempts = []
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = ssl.TLSVersion.TLSv1_3
        context.load_cert_chain(str(certificate), str(private_key))
        context.set_alpn_protocols(["http/1.1"])
        server = await asyncio.start_server(
            lambda r, w: legacy_peer(r, w, secret, attempts),
            "127.0.0.1", 0, ssl=context,
        )
        async with server:
            port = server.sockets[0].getsockname()[1]
            process = await asyncio.create_subprocess_exec(
                str(binary), "probe", "--host", "127.0.0.1",
                "--port", str(port), "--server-name", "localhost",
                "--ca", str(certificate), "--secret-file", str(secret_path),
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            try:
                stdout, stderr = await asyncio.wait_for(process.communicate(), 20)
            except TimeoutError:
                process.kill()
                await process.wait()
                raise
            assert process.returncode == 0, (stdout.decode(), stderr.decode())
            assert "TLS echo: OK" in stdout.decode(), (stdout, stderr)
            assert attempts == ["sync/1", "lab-packet/1"], attempts
        print("Legacy TCP 0.2.0 handshake and packet echo: OK")


if __name__ == "__main__":
    asyncio.run(test(Path(sys.argv[1]).resolve()))
