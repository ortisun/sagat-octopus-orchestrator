"""The cost of one captured logger call (``sdks/SPEC.md`` section 6.9, "The cost of a call"): an 8 KiB message redacted
with a secret set of 1,024 values, formatted, cut and sized. Run by hand, never by the normal test command or CI:

    SAGAT_STEP_LOG_BENCH=1 .venv/bin/python -m pytest -q -s tests/test_step_logs_bench.py

1,000 warm-up calls, then 10,000 measured on the monotonic clock; the p99 must be under 1 ms.
"""

from __future__ import annotations

import base64
import math
import os
import time

import pytest

from sagat_octopus_orchestrator.worker._step_logs import SecretSet, build_record


@pytest.mark.skipif(os.environ.get("SAGAT_STEP_LOG_BENCH") != "1", reason="run by hand: SAGAT_STEP_LOG_BENCH=1")
def test_one_captured_call_costs_under_a_millisecond_at_p99() -> None:
    secrets = SecretSet()
    secrets.add([base64.urlsafe_b64encode(os.urandom(12)) for _ in range(1024)])
    message = "x" * (8 * 1024 - 32) + " an ordinary line of text"
    fields = {"order": "o-123", "amount": 1999, "currency": "EUR"}

    def call(sequence: int) -> int:
        return build_record(sequence, time.time_ns() // 1_000_000, 20, message, (), fields, "", secrets).ByteSize()

    for n in range(1, 1001):
        call(n)
    samples: list[float] = []
    for n in range(1, 10_001):
        start = time.perf_counter_ns()
        call(n)
        samples.append((time.perf_counter_ns() - start) / 1e6)
    samples.sort()

    def at(q: float) -> float:
        return samples[min(len(samples) - 1, math.ceil(q * len(samples)) - 1)]

    print(
        f"python step-log call, 8 KiB message, 1,024 secrets: p50 {at(0.5):.4f} ms, p99 {at(0.99):.4f} ms, "
        f"max {at(1):.4f} ms"
    )
    assert at(0.99) < 1.0
