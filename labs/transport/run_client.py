"""Connect a Linux TUN device to the laboratory transport."""

from __future__ import annotations

import argparse
import asyncio

from .auth import read_secret_file
from .client import connect


async def _main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", type=int, default=9443)
    parser.add_argument("--server-name", required=True)
    parser.add_argument("--ca", help="custom CA file; omit for system trust store")
    parser.add_argument("--secret-file", required=True)
    parser.add_argument("--tun-name", default="labtun0")
    args = parser.parse_args()
    secret = read_secret_file(args.secret_file)

    from .linux_tun import LinuxTun

    device = LinuxTun(args.tun_name)
    tunnel = None
    try:
        tunnel = await connect(args.host, args.port, args.server_name, args.ca, secret)

        async def uplink() -> None:
            while True:
                await tunnel.send_batch(await device.read_batch())

        async def downlink() -> None:
            while True:
                await device.write_packet(await tunnel.receive_packet())

        async with asyncio.TaskGroup() as tasks:
            tasks.create_task(uplink())
            tasks.create_task(downlink())
    finally:
        if tunnel is not None:
            await tunnel.close()
        await device.close()


if __name__ == "__main__":
    asyncio.run(_main())
