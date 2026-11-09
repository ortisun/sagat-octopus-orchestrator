"""Doubles for the network failures a probe tells apart (feature 035, D035-150): a server whose certificate nobody
trusts, and a proxy that refuses the tunnel the way the relay refuses a destination the connection does not declare.

Each is a context manager that listens on the loopback and closes everything when the block ends: the tests run with
warnings as errors, and an unclosed socket is one.
"""

from __future__ import annotations

import contextlib
import datetime
import socket
import ssl
import threading
import time
from collections.abc import Callable, Generator
from contextlib import contextmanager
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

# What the relay answers to a `CONNECT` it refuses (`relay_double.FORBIDDEN`, `database-runner/src/lake/prepare.rs`).
REFUSED = b"HTTP/1.1 403 Forbidden\r\n\r\n"


@contextmanager
def _listening(handle: Callable[[socket.socket], None]) -> Generator[int]:
    """A TCP listener on the loopback that hands each connection to ``handle`` on a thread of its own."""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(8)
    listener.settimeout(0.05)
    stop = threading.Event()
    workers: list[threading.Thread] = []

    def serve() -> None:
        while not stop.is_set():
            try:
                connection, _ = listener.accept()
            except TimeoutError:
                continue
            worker = threading.Thread(target=_handled, args=(connection,), daemon=True)
            workers.append(worker)
            worker.start()

    def _handled(connection: socket.socket) -> None:
        with connection, contextlib.suppress(OSError):
            handle(connection)

    acceptor = threading.Thread(target=serve, daemon=True)
    acceptor.start()
    try:
        yield int(listener.getsockname()[1])
    finally:
        stop.set()
        acceptor.join(timeout=5)
        for worker in workers:
            worker.join(timeout=5)
        listener.close()


@contextmanager
def silent_server(hold: float) -> Generator[int]:
    """A server that takes the request, does not answer for ``hold`` seconds and then hangs up: what a token endpoint
    that never answers looks like, to a client with no time limit (it fails after ``hold``, and not with a timeout)."""

    def stay(connection: socket.socket) -> None:
        connection.recv(4096)
        time.sleep(hold)

    with _listening(stay) as port:
        yield port


@contextmanager
def refusing_proxy() -> Generator[int]:
    """A proxy that answers every `CONNECT` with 403, as the relay does for a destination it does not allow."""

    def refuse(connection: socket.socket) -> None:
        connection.recv(4096)
        connection.sendall(REFUSED)

    with _listening(refuse) as port:
        yield port


def _self_signed(directory: Path) -> tuple[str, str]:
    key = ec.generate_private_key(ec.SECP256R1())
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "127.0.0.1")])
    now = datetime.datetime.now(datetime.UTC)
    certificate = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=1))
        .sign(key, hashes.SHA256())
    )
    certificate_file, key_file = directory / "certificate.pem", directory / "key.pem"
    certificate_file.write_bytes(certificate.public_bytes(serialization.Encoding.PEM))
    key_file.write_bytes(
        key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption())
    )
    return str(certificate_file), str(key_file)


@contextmanager
def untrusted_tls_server(directory: Path) -> Generator[int]:
    """An HTTPS server on the loopback with a self-signed certificate, so no client that verifies one trusts it."""
    certificate, key = _self_signed(directory)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certificate, key)

    def serve(connection: socket.socket) -> None:
        try:
            with context.wrap_socket(connection, server_side=True) as secure:
                secure.recv(4096)
                secure.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
        except (ssl.SSLError, OSError):
            # a client that does not trust the certificate hangs up during the handshake: that is the point
            pass

    with _listening(serve) as port:
        yield port
