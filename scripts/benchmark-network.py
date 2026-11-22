#!/usr/bin/env python3
"""Measure an existing network path; never configure the VPN or routes."""
import argparse
import json
import math
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


def percentile(values, fraction):
    """Linear interpolation between ordered observations."""
    values = sorted(values)
    if not values:
        return None
    pos = (len(values) - 1) * fraction
    lo, hi = math.floor(pos), math.ceil(pos)
    return values[lo] + (values[hi] - values[lo]) * (pos - lo)


def latency(host, port, count, timeout):
    samples, errors = [], []
    for _ in range(count):
        start = time.perf_counter()
        try:
            with socket.create_connection((host, port), timeout=timeout):
                samples.append((time.perf_counter() - start) * 1000)
        except OSError as exc:
            errors.append(str(exc))
    return {"kind": "tcp_connect_ms", "host": host, "port": port,
            "samples": samples, "failures": len(errors), "errors": errors,
            "p50": percentile(samples, .5), "p95": percentile(samples, .95)}


def measure(args, protocol):
    command = [args.iperf, "-c", args.host, "-p", str(args.port), "-J", "-t", str(args.duration)]
    if protocol == "udp":
        command += ["-u", "-b", args.udp_bitrate]
    if args.reverse:
        command.append("-R")
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=args.timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"ok": False, "error": str(exc), "command": command}
    try:
        raw = json.loads(result.stdout)
    except json.JSONDecodeError:
        return {"ok": False, "error": "iperf3 returned invalid JSON", "exit_code": result.returncode,
                "stderr": result.stderr[-2000:], "command": command}
    if not isinstance(raw, dict):
        return {"ok": False, "error": "iperf3 JSON must be an object", "command": command}
    if result.returncode != 0 or raw.get("error"):
        return {"ok": False, "error": raw.get("error") or result.stderr[-2000:] or "iperf3 failed",
                "exit_code": result.returncode, "command": command}
    end = raw.get("end", {})
    if not isinstance(end, dict):
        return {"ok": False, "error": "iperf3 end summary must be an object", "command": command}
    return {"ok": True, "exit_code": result.returncode, "command": command,
            "summary": {key: end[key] for key in
                        ("sum", "sum_sent", "sum_received", "cpu_utilization_percent") if key in end},
            "iperf_json": raw}


def positive(value):
    value = int(value)
    if value <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", required=True, choices=("baseline", "tunnel"))
    parser.add_argument("--host", required=True, help="iperf3 server address or name")
    parser.add_argument("--port", type=positive, default=5201)
    parser.add_argument("--latency-host", required=True, help="TCP endpoint for connection timing")
    parser.add_argument("--latency-port", type=positive, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--iperf", default="iperf3", help="iperf3 executable")
    parser.add_argument("--protocol", choices=("tcp", "udp", "both"), default="both")
    parser.add_argument("--udp-bitrate", default="10M")
    parser.add_argument("--duration", type=positive, default=10)
    parser.add_argument("--repeats", type=positive, default=3)
    parser.add_argument("--latency-count", type=positive, default=10)
    parser.add_argument("--connect-timeout", type=positive, default=3)
    parser.add_argument("--timeout", type=positive, default=30, help="timeout per iperf3 run, seconds")
    parser.add_argument("--reverse", action="store_true", help="server sends data to client")
    args = parser.parse_args()
    if args.port > 65535 or args.latency_port > 65535:
        parser.error("ports must be at most 65535")
    if args.timeout <= args.duration:
        parser.error("--timeout must exceed --duration")
    protocols = ("tcp", "udp") if args.protocol == "both" else (args.protocol,)
    report = {"schema": 1, "label": args.label, "started_utc": datetime.now(timezone.utc).isoformat(),
              "settings": {"host": args.host, "port": args.port, "duration_seconds": args.duration,
                           "repeats": args.repeats, "udp_bitrate": args.udp_bitrate, "reverse": args.reverse},
              "latency": latency(args.latency_host, args.latency_port, args.latency_count, args.connect_timeout),
              "runs": []}
    for protocol in protocols:
        for repeat in range(args.repeats):
            print(f"{protocol} {repeat + 1}/{args.repeats}", file=sys.stderr, flush=True)
            sample = measure(args, protocol)
            sample.update(protocol=protocol, repeat=repeat + 1)
            report["runs"].append(sample)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return int(report["latency"]["failures"] > 0 or any(not run["ok"] for run in report["runs"]))


if __name__ == "__main__":
    sys.exit(main())
