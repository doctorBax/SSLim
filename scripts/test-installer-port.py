"""Regression: upgrades must probe the existing server TCP port, not 9443.

Extract and execute the actual Bash parser from the standalone installer;
a local systemd service, root privileges and an external VPS aren't needed.
"""
from pathlib import Path
import subprocess
import tempfile

installer = (Path(__file__).resolve().parent / "install-server.sh").read_text()
start = installer.index("service_tcp_port() {")
end = installer.index("\n}\n", start) + len("\n}")
parser = installer[start:end]


def check(unit: str, expected: str | None):
    with tempfile.TemporaryDirectory(prefix="sslim-unit-port-") as temporary:
        path = Path(temporary) / "sslim.service"
        path.write_text(unit)
        result = subprocess.run(
            ["bash", "-c", "set -euo pipefail\n" + parser + '\nservice_tcp_port "$1"',
             "test", str(path)],
            text=True, capture_output=True, timeout=5,
        )
        if expected is None:
            assert result.returncode != 0, (unit, result.stdout, result.stderr)
        else:
            assert result.returncode == 0, (unit, result.stderr)
            assert result.stdout.strip() == expected, (unit, result.stdout)


base = (
    "[Unit]\nDescription=SSLim\n[Service]\n"
    "ExecStart=/usr/local/bin/sslim server --host 0.0.0.0 --device tun "
    "--multi-tun --secrets-file /etc/sslim/clients.txt"
)
check(base + "\n", "9443")
check(base + " --port 8443\n", "8443")
check(base + " --port=10443 --udp-port 9543\n", "10443")
check(base + " --udp-port 9443\n", "9443")
check(base + " --port 0\n", None)
check(base + " --port 65536\n", None)
check("[Service]\nExecStartPre=/bin/true\n", None)
print("PASS: custom/legacy/default TCP ports survive upgrade health checks")
