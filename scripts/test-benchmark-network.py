"""Offline checks for measurement failures and result parsing; no sockets opened."""
import importlib.util
from pathlib import Path
import subprocess
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("benchmark", Path(__file__).with_name("benchmark-network.py"))
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class Measurements(unittest.TestCase):
    def setUp(self):
        self.args = SimpleNamespace(iperf="iperf3", host="test.invalid", port=5201,
                                    duration=1, timeout=3, reverse=False, udp_bitrate="10M")

    def test_summary_and_failures(self):
        for output, expected in [('[]', False), ('garbage', False),
                                 ('{"error":"denied"}', False),
                                 ('{"end":[]}', False),
                                 ('{"end":{"sum_received":{"bits_per_second":123}}}', True)]:
            with self.subTest(output=output), patch.object(benchmark.subprocess, "run", return_value=SimpleNamespace(stdout=output, stderr="", returncode=0)):
                result = benchmark.measure(self.args, "tcp")
                self.assertEqual(result["ok"], expected)
                if expected:
                    self.assertEqual(result["summary"]["sum_received"]["bits_per_second"], 123)
        for error in [FileNotFoundError("iperf3"), subprocess.TimeoutExpired("iperf3", 3)]:
            with patch.object(benchmark.subprocess, "run", side_effect=error):
                self.assertFalse(benchmark.measure(self.args, "udp")["ok"])

    def test_latency_failures_and_quantiles(self):
        self.assertEqual(benchmark.percentile([10, 30, 20], .5), 20)
        self.assertIsNone(benchmark.percentile([], .95))
        with patch.object(benchmark.socket, "create_connection", side_effect=OSError("unreachable")):
            result = benchmark.latency("test.invalid", 443, 2, 1)
        self.assertEqual(result["failures"], 2)
        self.assertIsNone(result["p50"])


if __name__ == "__main__":
    unittest.main()
