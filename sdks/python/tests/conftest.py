from __future__ import annotations

from collections.abc import AsyncIterator, Callable, Iterator
from typing import Any

import pytest

from sagat_octopus_orchestrator import ClientConfig, RetryPolicy, SagatClient, WorkerConfig, WorkerHost

from .fakes import FakeHttpServer, FakeWorkerService

FAST = RetryPolicy(initial_delay=0.005, max_delay=0.02)


@pytest.fixture
def http() -> Iterator[FakeHttpServer]:
    server = FakeHttpServer()
    try:
        yield server
    finally:
        server.close()


@pytest.fixture
async def make_client(http: FakeHttpServer) -> AsyncIterator[Callable[..., SagatClient]]:
    clients: list[SagatClient] = []

    def make(**extra: Any) -> SagatClient:
        settings: dict[str, Any] = {"api_url": http.base, "namespace": "payments", "token": "tok-1", "retry": FAST}
        settings.update(extra)
        client = SagatClient(ClientConfig(**settings))
        clients.append(client)
        return client

    yield make
    for client in clients:
        await client.aclose()


@pytest.fixture
async def fake() -> AsyncIterator[FakeWorkerService]:
    service = FakeWorkerService()
    await service.start()
    yield service
    await service.stop()


@pytest.fixture
async def make_host(fake: FakeWorkerService) -> AsyncIterator[Callable[..., WorkerHost]]:
    hosts: list[WorkerHost] = []

    def make(**extra: Any) -> WorkerHost:
        settings: dict[str, Any] = {
            "grpc_url": fake.url,
            "namespace": "payments",
            "token": "tok",
            "worker_name": "test-worker",
            "retry": FAST,
        }
        settings.update(extra)
        host = WorkerHost(WorkerConfig(**settings))
        hosts.append(host)
        return host

    yield make
    while hosts:
        await hosts.pop().stop(1.0)
