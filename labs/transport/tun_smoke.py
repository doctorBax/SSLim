"""Check that a Linux TUN device can be created without changing routes."""

from __future__ import annotations

import asyncio

from .linux_tun import LinuxTun


async def _run() -> None:
    device = LinuxTun("labcheck0")
    await device.close()


if __name__ == "__main__":
    asyncio.run(_run())
    print("Linux TUN create/close: OK")
