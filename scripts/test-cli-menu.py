#!/usr/bin/env python3
"""Smoke-test the actual Linux arrow-key TUI using a pseudoterminal.

No network, service changes or secrets are involved. Exercises a submenu and
back navigation with real VT cursor-key sequences, then quits cleanly.
"""
import os
import pty
import select
import subprocess
import sys
import time


def run(binary: str) -> None:
    master, slave = pty.openpty()
    child = subprocess.Popen(
        [binary],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        start_new_session=True,
        close_fds=True,
    )
    os.close(slave)
    output = b""

    def expect(text: str, timeout: float = 8.0) -> None:
        nonlocal output
        deadline = time.monotonic() + timeout
        token = text.encode("utf-8")
        while token not in output:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(
                    f"Menu did not contain {text!r}. Last output:\n"
                    + output[-3000:].decode("utf-8", "replace")
                )
            ready, _, _ = select.select([master], [], [], min(0.2, remaining))
            if ready:
                try:
                    output += os.read(master, 8192)
                except OSError as error:
                    raise AssertionError(f"Menu closed early: {error}; return={child.poll()}; output:\n" + output[-4000:].decode("utf-8", "replace")) from error

    try:
        expect("Главное меню · Linux")
        # The arrow down from first item selects "Ключи клиентов".
        os.write(master, b"\x1b[B")
        os.write(master, b"\r")
        expect("Linux · ключи клиентов")
        os.write(master, b"0")
        # The main heading appeared once before; wait for a second occurrence.
        output = b""
        expect("Главное меню · Linux")
        os.write(master, b"0")
        child.wait(timeout=5)
        if child.returncode != 0:
            raise AssertionError(f"CLI exited with {child.returncode}")
        print("PASS: arrows, submenu navigation, back and clean exit")
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=5)
        os.close(master)


if __name__ == "__main__":
    run(sys.argv[1] if len(sys.argv) > 1 else "target/release/sslim")
