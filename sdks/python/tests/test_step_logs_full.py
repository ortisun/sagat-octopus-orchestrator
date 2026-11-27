"""Step-log capture once the process's secret set is full (``sdks/SPEC.md`` sections 6.7 and 6.9).

A full set stops capture for the whole process and lives as long as it, so the scenario runs in a Python process of its
own: the test that pytest collects starts it and reads its result.
"""

from __future__ import annotations

import logging
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

from sagat_octopus_orchestrator import ActivityContext, WorkerEvent

from .conftest import FAST
from .fakes import FakeWorkerService, lease, reached, until

CHILD = "SAGAT_STEP_LOG_FULL_SET_CHILD"


@pytest.mark.skipif(os.environ.get(CHILD) == "1", reason="the process of its own runs the scenario itself")
def test_once_the_secret_set_is_full_nothing_more_is_sent_in_a_process_of_its_own() -> None:
    root = Path(__file__).resolve().parent.parent
    done = subprocess.run(
        [
            sys.executable,
            "-m",
            "pytest",
            "-q",
            "-p",
            "no:cacheprovider",
            f"{Path(__file__).resolve()}::test_the_full_set_scenario",
        ],
        cwd=root,
        env={**os.environ, CHILD: "1"},
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    assert "1 passed" in done.stdout, done.stdout


@pytest.mark.skipif(os.environ.get(CHILD) != "1", reason="runs only in a process of its own")
async def test_the_full_set_scenario(fake: FakeWorkerService, caplog: pytest.LogCaptureFixture) -> None:
    from sagat_octopus_orchestrator import WorkerConfig, WorkerHost
    from sagat_octopus_orchestrator._proto import worker_pb2 as pb

    fake.accepted_version = "1.7"
    fake.step_logs = True
    events: list[WorkerEvent] = []
    host = WorkerHost(
        WorkerConfig(
            grpc_url=fake.url,
            namespace="payments",
            token="tok",
            worker_name="full",
            transport="poll",
            max_inflight=1,
            retry=FAST,
            on_event=events.append,
        )
    )

    async def work(_input: Any, ctx: ActivityContext) -> Any:
        if ctx.attempt == 2:
            ctx.log.info("after the set filled")
            return {"second": True}
        for n in range(1, 11):
            ctx.log.info(f"before {n}")
        await ctx.secrets()  # the 1,025th value does not fit
        for n in range(1, 301):
            ctx.log.info(f"after {n}")
        # A tick of a second would find the records buffered before the set filled: it sends nothing now.
        assert not await reached(lambda: len(fake.of("AppendStepLogs")) > 0, 1.2)
        return {"first": True}

    host.register_activity("charge", work)
    fake.secrets = [pb.ResolvedSecret(name=f"s{n}", value=f"secret-{n:06d}".encode(), version=1) for n in range(1025)]
    try:
        with caplog.at_level(logging.WARNING, logger="sagat_octopus_orchestrator"):
            await host.start()
            fake.leases = [lease(task_id="run-1/charge/1"), lease(task_id="run-2/charge/1", attempt=2)]
            await until(lambda: len(fake.of("CompleteStep")) == 2, "both reports", 10.0)
    finally:
        await host.stop(1.0)
    assert fake.of("AppendStepLogs") == [], "no batch, no tick, no flush"
    first, second = (call.request for call in fake.of("CompleteStep"))
    assert list(first.step_logs.records) == []
    assert first.step_logs.highest_sequence == 310
    assert not second.HasField("step_logs"), "capture stopped for the process"
    assert [(event.reason, event.records) for event in events if event.type == "steplogs.dropped"] == [
        ("secret_set_full", 310)
    ]
    assert sum("secret set of the process is full" in record.getMessage() for record in caplog.records) == 1
