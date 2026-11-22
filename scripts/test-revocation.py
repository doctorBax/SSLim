#!/usr/bin/env python3
"""Exercise server key revocation using unprivileged loopback TLS sessions."""
import base64
import contextlib
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


def write_keys(path, entries):
    temporary = path.with_name("clients.new")
    temporary.write_text("".join(f"{label}:{base64.b64encode(key).decode()}\n" for label, key in entries.items()), encoding="ascii")
    os.replace(temporary, path)


def connect(port, key):
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE  # ephemeral test certificate on loopback
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    raw = socket.create_connection(("127.0.0.1", port), timeout=3)
    try:
        sock = context.wrap_socket(raw, server_hostname="localhost")
        now = str(int(time.time()))
        nonce = secrets.token_hex(16)
        message = f"vpn-bot-lab-transport-v1\0{now}\0{nonce}".encode()
        tag = hmac.new(key, message, hashlib.sha256).hexdigest()
        sock.sendall((f"POST /connect HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: sync/1\r\nX-Session-Time: {now}\r\nX-Session-Nonce: {nonce}\r\nX-Session-Tag: {tag}\r\n\r\n").encode())
        headers = b""
        while not headers.endswith(b"\r\n\r\n"):
            part = sock.recv(1)
            if not part or len(headers) >= 8192:
                raise AssertionError("incomplete response")
            headers += part
        if not headers.startswith(b"HTTP/1.1 101 "):
            sock.close()
            return None
        return sock
    except BaseException:
        raw.close()
        raise


def ping(sock):
    sock.settimeout(3)
    sock.sendall(b"\x02\x00\x00")
    result = b""
    while len(result) < 3:
        part = sock.recv(3 - len(result))
        if not part:
            raise AssertionError("session closed before pong")
        result += part
    assert result == b"\x03\x00\x00", f"unexpected pong: {result!r}"


def expect_closed(sock):
    sock.settimeout(3)
    try:
        data = sock.recv(1)
    except ssl.SSLEOFError:
        data = b""
    except socket.timeout as error:
        raise AssertionError("revoked session stayed open for 3 seconds") from error
    assert data == b"", f"unexpected data on revoked session: {data!r}"


def wait_connect(port, key):
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        sock = connect(port, key)
        if sock is not None:
            return sock
        time.sleep(0.1)
    raise AssertionError("new key was not accepted within 3 seconds")


def main(binary):
    with tempfile.TemporaryDirectory(prefix="sslim-revocation-") as tmp:
        directory = Path(tmp)
        cert, private, keys = (directory / name for name in ("cert.pem", "key.pem", "clients.txt"))
        subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-keyout", str(private), "-out", str(cert)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        alice, bob, rotated = (secrets.token_bytes(32) for _ in range(3))
        write_keys(keys, {"alice": alice, "bob": bob})
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        with (directory / "server.log").open("w+") as log:
            server = subprocess.Popen([str(Path(binary).resolve()), "server", "--host", "127.0.0.1", "--port", str(port), "--device", "echo", "--cert", str(cert), "--key", str(private), "--secrets-file", str(keys)], stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 5
                while True:
                    if server.poll() is not None:
                        raise AssertionError("server exited during startup")
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                            break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise AssertionError("server did not start")
                        time.sleep(0.05)
                with contextlib.ExitStack() as stack:
                    a = stack.enter_context(wait_connect(port, alice))
                    b = stack.enter_context(wait_connect(port, bob))
                    ping(a)
                    ping(b)
                    write_keys(keys, {"bob": bob})
                    expect_closed(a)
                    ping(b)
                    rejected = connect(port, alice)
                    if rejected is not None:
                        rejected.close()
                        raise AssertionError("revoked Alice key reconnected")
                    write_keys(keys, {"bob": rotated})
                    expect_closed(b)
                    rejected = connect(port, bob)
                    if rejected is not None:
                        rejected.close()
                        raise AssertionError("old Bob key reconnected")
                    new_b = stack.enter_context(wait_connect(port, rotated))
                    ping(new_b)
                    bad = keys.with_name("clients.new")
                    bad.write_text("invalid\n", encoding="ascii")
                    os.replace(bad, keys)
                    expect_closed(new_b)
                    rejected = connect(port, rotated)
                    if rejected is not None:
                        rejected.close()
                        raise AssertionError("key accepted while file is invalid")
                    write_keys(keys, {"bob": rotated})
                    restored = stack.enter_context(wait_connect(port, rotated))
                    ping(restored)
                print("Key revocation, rotation and fail-closed recovery passed.")
            except BaseException:
                log.flush()
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("Usage: test-revocation.py /path/to/sslim")
    main(sys.argv[1])
