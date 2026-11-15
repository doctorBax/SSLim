"""Loopback interoperability checks and an optional server CPU comparison."""

import argparse
import asyncio
import os
import ssl
import subprocess
import tempfile
import time
from pathlib import Path

from labs.transport.client import connect
from labs.transport.device import EchoPacketDevice
from labs.transport.server import serve
from labs.transport.smoke import _ephemeral_certificate


def make_packet(length, sequence):
    packet = bytearray(length)
    packet[0] = 0x45
    packet[2:4] = length.to_bytes(2, "big")
    packet[8] = 64
    packet[9] = 17
    packet[12:20] = bytes([127, 0, 0, 1] * 2)
    packet[20:24] = sequence.to_bytes(4, "big")
    return bytes(packet)


async def exchange(port, cert, secret, length, count):
    packets = [make_packet(length, sequence) for sequence in range(32)]
    tunnel = await connect("127.0.0.1", port, "localhost", str(cert), secret)
    try:
        async def send():
            for _ in range(count // len(packets)):
                await tunnel.send_batch(packets)

        async def receive():
            for index in range(count):
                if await tunnel.receive_packet() != packets[index % len(packets)]:
                    raise RuntimeError("packet payload or order changed")

        async with asyncio.timeout(30), asyncio.TaskGroup() as tasks:
            tasks.create_task(send())
            tasks.create_task(receive())
    finally:
        await tunnel.close()


async def public_status(port, cert, request):
    reader, writer = await asyncio.open_connection(
        "127.0.0.1", port,
        ssl=ssl.create_default_context(cafile=str(cert)),
        server_hostname="localhost",
    )
    try:
        writer.write(request)
        await writer.drain()
        return await reader.read()
    finally:
        writer.close()
        await writer.wait_closed()


async def verify(rust, cert, private_key, secret_file, port):
    secret = secret_file.read_bytes()
    await exchange(port, cert, secret, 1280, 32)
    try:
        unauthorized = await connect("127.0.0.1", port, "localhost", str(cert), os.urandom(32))
    except ConnectionError:
        pass
    else:
        await unauthorized.close()
        raise RuntimeError("Rust server accepted an invalid secret")
    async with asyncio.timeout(5):
        head = await public_status(port, cert, b"HEAD / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        if not head.startswith(b"HTTP/1.1 200 OK\r\n") or not head.endswith(b"\r\n\r\n"):
            raise RuntimeError("unexpected HEAD response")
        malformed = await public_status(port, cert, b"GET / HTTP/1.1\r\nHost: localhost\r\nHost: duplicate\r\n\r\n")
        if not malformed.startswith(b"HTTP/1.1 400 Bad Request\r\n"):
            raise RuntimeError("duplicate header accepted")
    server = await serve(
        "127.0.0.1", 0, str(cert), str(private_key), secret, EchoPacketDevice
    )
    async with server:
        python_port = server.sockets[0].getsockname()[1]
        probe = await asyncio.create_subprocess_exec(
            str(rust), "probe", "--host", "127.0.0.1", "--port", str(python_port),
            "--server-name", "localhost", "--ca", str(cert), "--secret-file", str(secret_file)
        )
        async with asyncio.timeout(10):
            if await probe.wait():
                raise RuntimeError("Rust probe failed against the Python server")
    print("Python → Rust, Rust → Python, bad secret, HEAD, duplicate header: OK", flush=True)


async def python_echo(cert, key, secret_file):
    server = await serve("127.0.0.1", 0, cert, key, Path(secret_file).read_bytes(), EchoPacketDevice)
    async with server:
        print(f"listening on 127.0.0.1:{server.sockets[0].getsockname()[1]} (echo)", flush=True)
        await server.serve_forever()


def server_cpu(pid):
    stat = Path(f"/proc/{pid}/stat").read_text().rpartition(")")[2].split()
    return (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK")


def start_server(command, cert, private_key, secret_file):
    if command is None:
        code = "from scripts.verify import asyncio, python_echo; import sys; asyncio.run(python_echo(*sys.argv[1:]))"
        command = [os.sys.executable, "-u", "-c", code, str(cert), str(private_key), str(secret_file)]
    else:
        command = [*command, "--host", "127.0.0.1", "--port", "0", "--device", "echo",
                   "--cert", str(cert), "--key", str(private_key), "--secret-file", str(secret_file)]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, text=True)
    line = proc.stdout.readline()
    try:
        return proc, int(line.split("127.0.0.1:")[1].split(" ")[0])
    except (ValueError, IndexError):
        stop_server(proc)
        raise RuntimeError(f"server failed to start: {line!r}") from None


def stop_server(proc):
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    proc.stdout.close()


async def main(args):
    rust = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="govpn-verify-") as directory:
        root = Path(directory)
        cert, private_key = _ephemeral_certificate(root)
        secret_file = root / "session.bin"
        with open(secret_file, "xb", opener=lambda path, flags: os.open(path, flags, 0o600)) as output:
            output.write(os.urandom(32))
        rust_server, rust_port = start_server([str(rust), "server"], cert, private_key, secret_file)
        try:
            await verify(rust, cert, private_key, secret_file, rust_port)
            if not args.bench:
                return
            python_server, python_port = start_server(
                None,
                cert, private_key, secret_file,
            )
            try:
                secret = secret_file.read_bytes()
                for length in (64, 1280):
                    for label, proc, port in (
                        ("Python", python_server, python_port), ("Rust", rust_server, rust_port)
                    ):
                        await exchange(port, cert, secret, length, 4096)
                        before = server_cpu(proc.pid)
                        start = time.perf_counter()
                        await exchange(port, cert, secret, length, 131072)
                        elapsed = time.perf_counter() - start
                        cpu = server_cpu(proc.pid) - before
                        print(
                            f"{label} {length:4} B  {131072 / elapsed:8.0f} packets/s"
                            f"  {cpu:.2f} server CPU s  {elapsed:.2f} wall s",
                            flush=True,
                        )
            finally:
                stop_server(python_server)
        finally:
            stop_server(rust_server)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--bench", action="store_true")
    asyncio.run(main(parser.parse_args()))
