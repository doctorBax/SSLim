"""Run a standalone laboratory server."""

from __future__ import annotations

import argparse
import asyncio

from .auth import read_secret_file
from .device import EchoPacketDevice, IcmpResponderDevice
from .server import serve


async def _main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=9443)
    parser.add_argument("--cert", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--secret-file", required=True)
    parser.add_argument("--device", choices=("echo", "icmp", "tun"), default="echo")
    parser.add_argument("--tun-name", default="labtun0")
    args = parser.parse_args()

    if args.device == "tun":
        from .linux_tun import LinuxTun

        device_factory = lambda: LinuxTun(args.tun_name)
    elif args.device == "icmp":
        device_factory = IcmpResponderDevice
    else:
        device_factory = EchoPacketDevice
    server = await serve(
        args.host,
        args.port,
        args.cert,
        args.key,
        read_secret_file(args.secret_file),
        device_factory,
    )
    print(f"laboratory server listening on {args.host}:{args.port} ({args.device})")
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(_main())
