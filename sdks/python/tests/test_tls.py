"""TLS and mTLS to the REST API and to the worker service, with throw-away certificates made by ``openssl``."""

from __future__ import annotations

import shutil
import ssl
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import grpc
import grpc.aio
import pytest

from sagat_octopus_orchestrator import (
    ClientConfig,
    RetryPolicy,
    SagatClient,
    SagatError,
    TlsOptions,
    WorkerConfig,
    WorkerHost,
)

from .fakes import FakeHttpServer, FakeWorkerService, lease, reply, until
from .test_client import run_body

pytestmark = pytest.mark.skipif(shutil.which("openssl") is None, reason="needs the openssl command")


@dataclass
class Pair:
    cert: bytes
    key: bytes


def make_pair(directory: Path, name: str) -> Pair:
    """A self-signed certificate for ``localhost`` and 127.0.0.1 that is its own trust root."""
    key, cert, config = directory / f"{name}.key", directory / f"{name}.pem", directory / f"{name}.cnf"
    config.write_text(
        "[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=localhost\n"
        "[ext]\nsubjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:TRUE\n"
    )
    run = subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "2",
            "-config",
            str(config),
            "-keyout",
            str(key),
            "-out",
            str(cert),
        ],
        capture_output=True,
        check=False,
    )
    if run.returncode != 0:
        pytest.skip(f"openssl could not make a certificate: {run.stderr.decode()[:200]}")
    return Pair(cert.read_bytes(), key.read_bytes())


@pytest.fixture
def server_pair(tmp_path: Path) -> Pair:
    return make_pair(tmp_path, "server")


@pytest.fixture
def client_pair(tmp_path: Path) -> Pair:
    return make_pair(tmp_path, "client")


def secure_http(http: FakeHttpServer, pair: Pair, trust: Pair | None, tmp_path: Path) -> str:
    (tmp_path / "s.pem").write_bytes(pair.cert)
    (tmp_path / "s.key").write_bytes(pair.key)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(str(tmp_path / "s.pem"), str(tmp_path / "s.key"))
    if trust is not None:
        context.verify_mode = ssl.CERT_REQUIRED
        context.load_verify_locations(cadata=trust.cert.decode())
    server: Any = http._server
    server.socket = context.wrap_socket(server.socket, server_side=True)
    return http.base.replace("http://", "https://").replace("127.0.0.1", "localhost")


@pytest.fixture
def secure_rest(http: FakeHttpServer, server_pair: Pair, tmp_path: Path) -> str:
    return secure_http(http, server_pair, None, tmp_path)


async def test_the_rest_client_trusts_the_roots_it_is_given_and_refuses_a_server_it_cannot_verify(
    http: FakeHttpServer, secure_rest: str, server_pair: Pair
) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    trusting = SagatClient(
        ClientConfig(api_url=secure_rest, namespace="p", token="t", tls=TlsOptions(ca=server_pair.cert))
    )
    assert (await trusting.get("0190aaaa-0000-7000-8000-000000000001")).status == "RUNNING"
    await trusting.aclose()

    strict = SagatClient(ClientConfig(api_url=secure_rest, namespace="p", token="t", retry=RetryPolicy(max_attempts=1)))
    with pytest.raises(SagatError) as caught:
        await strict.get("0190aaaa-0000-7000-8000-000000000001")  # the system roots do not know this certificate
    assert caught.value.code == "TRANSPORT"
    await strict.aclose()


async def test_a_redirect_is_not_followed_over_tls_either(
    http: FakeHttpServer, secure_rest: str, server_pair: Pair
) -> None:
    elsewhere = FakeHttpServer()
    try:
        elsewhere.serve(lambda _seen: reply(200, run_body()))
        http.serve(lambda _seen: reply(307, "", location=elsewhere.base + "/runs/r"))
        client = SagatClient(
            ClientConfig(api_url=secure_rest, namespace="p", token="t", tls=TlsOptions(ca=server_pair.cert))
        )
        with pytest.raises(SagatError) as caught:
            await client.start("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", {})
        assert (caught.value.code, caught.value.status) == ("PROTOCOL", 307)
        assert len(http.seen) == 1 and elsewhere.seen == []
        await client.aclose()
    finally:
        elsewhere.close()


async def test_the_rest_client_presents_its_certificate_for_mtls(
    http: FakeHttpServer, server_pair: Pair, client_pair: Pair, tmp_path: Path
) -> None:
    base = secure_http(http, server_pair, client_pair, tmp_path)
    http.serve(lambda _seen: reply(200, run_body()))
    tls = TlsOptions(ca=server_pair.cert, cert=client_pair.cert, key=client_pair.key)
    client = SagatClient(ClientConfig(api_url=base, namespace="p", token="t", tls=tls))
    assert (await client.get("0190aaaa-0000-7000-8000-000000000001")).run_id
    await client.aclose()
    without = SagatClient(
        ClientConfig(
            api_url=base,
            namespace="p",
            token="t",
            tls=TlsOptions(ca=server_pair.cert),
            retry=RetryPolicy(max_attempts=1),
        )
    )
    with pytest.raises(SagatError):
        await without.get("0190aaaa-0000-7000-8000-000000000001")
    await without.aclose()


async def secure_grpc(service: FakeWorkerService, server_pair: Pair, trust: Pair | None) -> grpc.aio.Server:
    server = grpc.aio.server()
    from typing import cast

    from sagat_octopus_orchestrator._proto import worker_pb2_grpc as pb_grpc

    cast(Any, pb_grpc).add_WorkerServiceServicer_to_server(service, server)
    credentials = grpc.ssl_server_credentials(
        [(server_pair.key, server_pair.cert)],
        root_certificates=trust.cert if trust else None,
        require_client_auth=trust is not None,
    )
    service.port = server.add_secure_port("localhost:0", credentials)
    await server.start()
    return server


@pytest.mark.parametrize("mutual", [False, True])
async def test_the_worker_host_connects_over_tls_and_over_mtls(
    server_pair: Pair, client_pair: Pair, mutual: bool
) -> None:
    service = FakeWorkerService()
    server = await secure_grpc(service, server_pair, client_pair if mutual else None)
    tls = TlsOptions(
        ca=server_pair.cert, cert=client_pair.cert if mutual else None, key=client_pair.key if mutual else None
    )
    host = WorkerHost(
        WorkerConfig(
            grpc_url=f"https://localhost:{service.port}",
            namespace="p",
            token="t",
            worker_name="w",
            tls=tls,
            retry=RetryPolicy(max_attempts=1),
        )
    )
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    service.leases = [lease()]
    try:
        await host.start()
        await until(lambda: len(service.of("CompleteStep")) == 1, "a task served over TLS")
    finally:
        await host.stop(2)
        await server.stop(None)


async def test_a_worker_that_does_not_trust_the_server_does_not_register(server_pair: Pair) -> None:
    service = FakeWorkerService()
    server = await secure_grpc(service, server_pair, None)
    host = WorkerHost(
        WorkerConfig(
            grpc_url=f"https://localhost:{service.port}",
            namespace="p",
            token="t",
            worker_name="w",
            retry=RetryPolicy(max_attempts=1),
            timeout=3,
        )
    )
    host.register_activity("charge", lambda _i, _c: 1)
    try:
        with pytest.raises(SagatError) as caught:
            await host.start()
        assert caught.value.code == "TRANSPORT" and not service.calls
    finally:
        await server.stop(None)


def test_tls_options_can_be_read_from_files(tmp_path: Path) -> None:
    (tmp_path / "ca.pem").write_bytes(b"CA")
    options = TlsOptions.from_files(ca=tmp_path / "ca.pem")
    assert options.ca == b"CA" and options.cert is None
