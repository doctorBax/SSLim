# Protocol compatibility

SSLim transports are versioned by their wire behavior, not by the application
release number. A working connection requires a matching secret, a trusted
server certificate and a compatible transport listener.

| Client releases | TCP Upgrade | New server (this branch) | Old server |
| --- | --- | --- | --- |
| v0.1.0, v0.2.0 | `lab-packet/1` | Accepted; response uses `lab-packet/1` | Native support |
| v0.3.0–v0.6.1 | `sync/1` | Accepted; response uses `sync/1` | Current Linux/Windows/Android clients retry `lab-packet/1` on HTTP 404 |

The TCP packet format, TLS 1.3 validation and HMAC domain are the same.
Both Upgrade markers are authenticated identically; unknown markers and invalid
secrets are refused. The client must use its original secret and the same CA.

## Links and Android

The `sslim://` parameter names in releases v0.3.0 through v0.6.1 are
unchanged. A link may contain `u=lab-packet/1` to select a historical TCP
marker and `ca=` for the base64-encoded PEM trust certificate. When sharing a
profile, Android Blade must retain **both** fields, not only host and key.

Android display labels may contain Cyrillic text and spaces. The Android
exporter percent-encodes the `n=` label for valid URIs. Android restores the
display name on import; Linux/Windows instead use the host as a safe local
profile name when that encoded label is not a valid filename, without changing
the server, secret, CA or TCP marker.


## UDP / QUIC is a different compatibility axis

- Experimental QUIC v2 (`sslim/2`) remains a separate listener using
  `sslim quic server` and is not silently transformed into QUIC v3.
- QUIC v3 (`sslim/3`) is available from v0.6.0. The regular TCP server accepts
  it only when `--udp-port` is explicitly enabled (new v0.6.1 installations
  enable it by default, upgrades preserve the old service flags).
- Old TCP links do **not** become UDP/QUIC v3 links by magic. Android's Auto
  transport uses authenticated TCP fallback when UDP is unavailable.
- A server running only TCP cannot accept UDP merely because the client is new.

## Regression tests

- `scripts/test-legacy-tcp.py`: current Linux TCP client to a v0.2-style
  loopback server, including the HTTP 404 compatibility retry.
- `scripts/test-legacy-clients.py`: authenticated historic and current TCP
  wire formats to the current Linux server, including frame echo and rejection
  of incorrect secrets/unknown markers.
- `scripts/test-android-motion.py`: Android source contract that shared
  profiles retain the Upgrade marker and private CA alongside UI checks.

These are **wire-format compatibility tests**. They do not yet certify every
historical compiled Linux/Windows binary or Android APK on real devices. Test
those separately before claiming full release-to-release interoperability.
