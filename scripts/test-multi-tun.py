"""CI smoke test. Run in a disposable network namespace with CAP_NET_ADMIN."""

import hashlib
import hmac
import ipaddress
import os
from pathlib import Path
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import time


def checksum(data):
    words = struct.unpack(f"!{len(data) // 2}H", data)
    value = sum(words)
    while value >> 16:
        value = (value & 0xFFFF) + (value >> 16)
    return ~value & 0xFFFF


def open_session(context, key):
    raw = socket.create_connection(("127.0.0.1", 9443), timeout=3)
    sock = context.wrap_socket(raw, server_hostname="localhost")
    stamp, nonce = str(int(time.time())), os.urandom(16).hex()
    tag = hmac.new(key, f"vpn-bot-lab-transport-v1\0{stamp}\0{nonce}".encode(), hashlib.sha256).hexdigest()
    sock.sendall((
        "POST /connect HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: sync/1\r\n"
        f"X-Session-Time: {stamp}\r\nX-Session-Nonce: {nonce}\r\nX-Session-Tag: {tag}\r\n"
        "X-Session-Address: auto\r\n\r\n"
    ).encode())
    stream = sock.makefile("rb")
    with stream:
        assert stream.readline().startswith(b"HTTP/1.1 101 ")
        address = None
        for line in iter(stream.readline, b"\r\n"):
            assert line, "incomplete response"
            if line.startswith(b"X-Session-Address: "):
                address = ipaddress.IPv4Address(line.split(b":", 1)[1].strip().decode())
        assert address is not None
        return sock, address


def send_ping(sock, address, sequence):
    destination = ipaddress.IPv4Address("10.253.0.1").packed
    icmp = struct.pack("!BBHHH", 8, 0, 0, 123, sequence)
    icmp = icmp[:2] + struct.pack("!H", checksum(icmp)) + icmp[4:]
    header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 28, sequence, 0, 64, 1, 0, address.packed, destination)
    header = header[:10] + struct.pack("!H", checksum(header)) + header[12:]
    sock.sendall(struct.pack("!BH", 1, 28) + header + icmp)


def check_reply(sock, address, sequence):
    with sock.makefile("rb") as stream:
        frame_type, length = struct.unpack("!BH", stream.read(3))
        packet = stream.read(length)
        assert frame_type == 1 and len(packet) == 28
        assert packet[16:20] == address.packed
        assert packet[20] == 0 and packet[26:28] == struct.pack("!H", sequence)


def main():
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as tmp:
        folder = Path(tmp)
        cert, tls_key, session_key = (folder / name for name in ("cert.pem", "key.pem", "secret.bin"))
        key = os.urandom(32)
        session_key.write_bytes(key)
        subprocess.run([
            "openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
            "-nodes", "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost",
            "-addext", "basicConstraints=critical,CA:FALSE", "-keyout", str(tls_key), "-out", str(cert),
        ], check=True)
        context = ssl.create_default_context(cafile=str(cert))
        context.minimum_version = ssl.TLSVersion.TLSv1_3
        with subprocess.Popen([
            str(binary), "server", "--device", "tun", "--multi-tun", "--cert", str(cert),
            "--key", str(tls_key), "--secret-file", str(session_key),
        ]) as server:
            try:
                deadline = time.monotonic() + 5
                while True:
                    try:
                        with socket.create_connection(("127.0.0.1", 9443), timeout=1):
                            break
                    except OSError:
                        if server.poll() is not None or time.monotonic() >= deadline:
                            raise
                        time.sleep(0.05)
                first, first_ip = open_session(context, key)
                with first:
                    second, second_ip = open_session(context, key)
                    with second:
                        assert first_ip != second_ip
                        send_ping(first, first_ip, 1)
                        send_ping(second, second_ip, 2)
                        check_reply(second, second_ip, 2)
                        check_reply(first, first_ip, 1)
                print("Two concurrent sessions on the same key passed independent IPv4 traffic.")
            finally:
                server.terminate()


if __name__ == "__main__":
    main()
