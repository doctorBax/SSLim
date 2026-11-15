"""Client authentication on top of a verified TLS connection."""

from __future__ import annotations

import hashlib
import hmac
import secrets
import time
from collections import deque
from pathlib import Path


AUTH_WINDOW_SECONDS = 60
_REPLAY_LIMIT = 4096
_DOMAIN = b"vpn-bot-lab-transport-v1\x00"


def new_nonce() -> str:
    return secrets.token_hex(16)


def read_secret_file(path: str) -> bytes:
    secret = Path(path).read_bytes()
    if len(secret) < 32:
        raise ValueError("authentication secret file must contain at least 32 bytes")
    return secret


def make_tag(secret: bytes, timestamp: int, nonce: str) -> str:
    if len(secret) < 32:
        raise ValueError("authentication secret must be at least 32 bytes")
    if len(nonce) != 32 or any(c not in "0123456789abcdef" for c in nonce):
        raise ValueError("nonce must be 16 lowercase hexadecimal bytes")
    message = _DOMAIN + str(timestamp).encode("ascii") + b"\x00" + nonce.encode("ascii")
    return hmac.new(secret, message, hashlib.sha256).hexdigest()


class ReplayGuard:
    def __init__(self) -> None:
        self._seen: set[str] = set()
        self._expirations: deque[tuple[float, str]] = deque()

    def verify(self, secret: bytes, timestamp: str, nonce: str, tag: str) -> bool:
        if not 1 <= len(timestamp) <= 12 or not timestamp.isdecimal() or len(tag) != 64:
            return False
        try:
            parsed_time = int(timestamp)
            if abs(int(time.time()) - parsed_time) > AUTH_WINDOW_SECONDS:
                return False
            expected = make_tag(secret, parsed_time, nonce)
            if not hmac.compare_digest(expected, tag):
                return False
        except (TypeError, ValueError):
            return False

        now = time.monotonic()
        while self._expirations and self._expirations[0][0] <= now:
            _, expired = self._expirations.popleft()
            self._seen.remove(expired)
        if nonce in self._seen or len(self._seen) >= _REPLAY_LIMIT:
            return False
        # A client clock at +60 s can leave the request valid for another 120 s.
        self._seen.add(nonce)
        self._expirations.append((now + 2 * AUTH_WINDOW_SECONDS + 1, nonce))
        return True
