#!/usr/bin/env bash
# Exercise IPv4 and IPv6 over a Rust-to-Rust tunnel in two isolated network namespaces.
set -euo pipefail

binary=$(realpath "${1:?usage: bash scripts/tun-check.sh path/to/govpn-lab}")
exec unshare --net bash -s -- "$binary" <<'CHECK'
set -euo pipefail
binary=$1
work=$(mktemp -d)
namespace= server= client=
cleanup() {
    for pid in "$client" "$server" "$namespace"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    rm -f "$work"/{cert.pem,key.pem,session.bin,namespace.ready,server.log,client.log}
    rmdir "$work"
}
trap cleanup EXIT

python3 - "$work" <<'PY'
import os
import sys
from pathlib import Path
from labs.transport.smoke import _ephemeral_certificate

root = Path(sys.argv[1])
_ephemeral_certificate(root)
(root / "session.bin").write_bytes(os.urandom(32))
PY

wait_file() {
    for _ in {1..60}; do
        if [[ -s "$1" ]]; then return; fi
        sleep 0.05
    done
    echo "timed out waiting for $1" >&2
    return 1
}
wait_device() {
    for _ in {1..60}; do
        if "$@" >/dev/null 2>&1; then return; fi
        sleep 0.05
    done
    cat "$work/client.log" >&2
    return 1
}

unshare --net sh -c 'echo ready > "$1"; exec sleep 120' sh "$work/namespace.ready" &
namespace=$!
wait_file "$work/namespace.ready"
remote() { nsenter --target "$namespace" --net -- "$@"; }

ip link add govpn-test-a type veth peer name govpn-test-b
ip link set govpn-test-b netns "$namespace"
ip addr add 198.18.0.1/30 dev govpn-test-a
ip link set govpn-test-a up
remote ip addr add 198.18.0.2/30 dev govpn-test-b
remote ip link set govpn-test-b up

remote "$binary" server --host 198.18.0.2 --port 29443 --device tun --tun-name govpn-tun-b \
    --cert "$work/cert.pem" --key "$work/key.pem" --secret-file "$work/session.bin" \
    >"$work/server.log" 2>&1 &
server=$!
wait_file "$work/server.log"

"$binary" client --host 198.18.0.2 --port 29443 --server-name localhost --tun-name govpn-tun-a \
    --ca "$work/cert.pem" --secret-file "$work/session.bin" >"$work/client.log" 2>&1 &
client=$!
wait_device ip link show govpn-tun-a
wait_device remote ip link show govpn-tun-b

ip addr add 198.19.0.1/30 dev govpn-tun-a
ip -6 addr add fd00:19::1/64 dev govpn-tun-a nodad
ip link set govpn-tun-a up
remote ip addr add 198.19.0.2/30 dev govpn-tun-b
remote ip -6 addr add fd00:19::2/64 dev govpn-tun-b nodad
remote ip link set govpn-tun-b up
ping -c 3 -W 2 198.19.0.2
ping -6 -c 3 -W 2 fd00:19::2
echo 'Linux TUN, IPv4 and IPv6 round trips: OK'
CHECK
