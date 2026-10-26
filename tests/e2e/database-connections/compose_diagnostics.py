"""Bounded, allowlisted diagnostics for the database Compose fixture."""
from enum import Enum
import json
import os
import re
import selectors
import subprocess
import sys
import time


MAX_CAPTURE_BYTES = 64 * 1024
DIAGNOSTIC_SECONDS = 5.0
_REAP_RESERVE_SECONDS = 0.25
SERVICES = ("openbao-volume-init", "openbao", "target-postgres", "dev-idp", "postgres", "migrate", "nats")
_CONTAINER_ID = re.compile(rb"[0-9a-f]{12,64}")
_EXIT_CODE = re.compile(rb"(?:0|[1-9][0-9]{0,2})")
_PROJECT = re.compile(r"[a-z0-9][a-z0-9_-]{0,62}")
_STATES = {"created", "running", "paused", "restarting", "removing", "exited", "dead"}
_HEALTH = {"none", "starting", "healthy", "unhealthy"}
_INSPECT_FORMAT = (
    '{{json (index .Config.Labels "com.docker.compose.service")}}\t'
    '{{json .State.Status}}\t{{.State.ExitCode}}\t{{if .State.Health}}'
    '{{json .State.Health.Status}}{{else}}"none"{{end}}'
)


class FailureCategory(Enum):
    DAEMON_UNAVAILABLE = "daemon_unavailable"
    HOST_PORT_UNAVAILABLE = "host_port_unavailable"
    NETWORK_SUBNET_CONFLICT = "network_subnet_conflict"
    IMAGE_UNAVAILABLE = "image_unavailable"
    DEPENDENCY_FAILED = "dependency_failed"
    CONTAINER_RUNTIME_FAILED = "container_runtime_failed"
    MOUNT_FAILED = "mount_failed"
    HOST_RESOURCE_EXHAUSTED = "host_resource_exhausted"
    UNCLASSIFIED = "unclassified"


_PATTERNS = {
    FailureCategory.DAEMON_UNAVAILABLE: (
        b"cannot connect to the docker daemon",
        b"is the docker daemon running",
        b"error during connect",
    ),
    FailureCategory.HOST_PORT_UNAVAILABLE: (
        b"port is already allocated",
        b"address already in use",
        b"failed to bind host port",
    ),
    FailureCategory.NETWORK_SUBNET_CONFLICT: (
        b"pool overlaps with other one on this address space",
        b"invalid pool request",
    ),
    FailureCategory.IMAGE_UNAVAILABLE: (
        b"manifest unknown",
        b"pull access denied",
        b"no matching manifest",
        b"failed to resolve reference",
    ),
    FailureCategory.DEPENDENCY_FAILED: (
        b"dependency failed to start",
        b"dependency failed to complete successfully",
        b"didn't complete successfully",
        b"is unhealthy",
    ),
    FailureCategory.CONTAINER_RUNTIME_FAILED: (
        b"failed to create task for container",
        b"oci runtime create failed",
    ),
    FailureCategory.MOUNT_FAILED: (
        b"invalid mount config",
        b"error while creating mount source path",
        b"failed to mount",
    ),
    FailureCategory.HOST_RESOURCE_EXHAUSTED: (
        b"no space left on device",
        b"cannot allocate memory",
        b"too many open files",
    ),
}


def classify_start_failure(stderr):
    if not isinstance(stderr, bytes) or len(stderr) > MAX_CAPTURE_BYTES:
        return FailureCategory.UNCLASSIFIED
    normalized = stderr.lower()
    matched = {
        category
        for category, patterns in _PATTERNS.items()
        if any(pattern in normalized for pattern in patterns)
    }
    if len(matched) == 1:
        return matched.pop()
    return FailureCategory.UNCLASSIFIED


def _kill_and_reap(process, deadline, monotonic):
    try:
        process.kill()
    except (OSError, ProcessLookupError):
        pass
    try:
        process.wait(timeout=max(0.0, deadline - monotonic()))
    except (OSError, subprocess.TimeoutExpired):
        pass


def _run_probe(argv, env, cwd, deadline, popen_factory, selector_factory, monotonic):
    execution_deadline = deadline - _REAP_RESERVE_SECONDS
    if execution_deadline <= monotonic():
        return None
    process = None
    completed = False
    try:
        process = popen_factory(
            argv,
            env=env,
            cwd=cwd,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            stdin=subprocess.DEVNULL,
        )
    except Exception:
        return None
    output = bytearray()
    try:
        if process.stdout is None:
            return None
        descriptor = process.stdout.fileno()
        os.set_blocking(descriptor, False)
        with selector_factory() as selector:
            selector.register(descriptor, selectors.EVENT_READ)
            eof = False
            while not eof:
                remaining = execution_deadline - monotonic()
                if remaining <= 0:
                    return None
                events = selector.select(remaining)
                if not events:
                    return None
                for key, _ in events:
                    while True:
                        try:
                            chunk = os.read(
                                key.fd,
                                min(8192, MAX_CAPTURE_BYTES + 1 - len(output)),
                            )
                        except BlockingIOError:
                            break
                        if not chunk:
                            eof = True
                            break
                        output.extend(chunk)
                        if len(output) > MAX_CAPTURE_BYTES:
                            return None
        remaining = execution_deadline - monotonic()
        if remaining <= 0:
            return None
        if process.wait(timeout=remaining):
            return None
        if monotonic() > execution_deadline:
            return None
        completed = True
        return bytes(output)
    except Exception:
        return None
    finally:
        try:
            if process.stdout is not None:
                process.stdout.close()
        finally:
            if not completed:
                _kill_and_reap(process, deadline, monotonic)


def _parse_inspect(stdout, expected_ids):
    lines = stdout.splitlines()
    if len(lines) != len(expected_ids):
        return None
    observed = {}
    for line in lines:
        if len(line) > 512:
            return None
        parts = line.split(b"\t")
        if len(parts) != 4:
            return None
        try:
            service = json.loads(parts[0])
            state = json.loads(parts[1])
            if not _EXIT_CODE.fullmatch(parts[2]):
                return None
            exit_code = int(parts[2])
            health = json.loads(parts[3])
        except (UnicodeDecodeError, ValueError, TypeError, json.JSONDecodeError):
            return None
        if not all(isinstance(value, str) for value in (service, state, health)):
            return None
        if service not in SERVICES or service in observed:
            return None
        if state not in _STATES or health not in _HEALTH or not 0 <= exit_code <= 255:
            return None
        observed[service] = (state, str(exit_code), health)
    return observed


def _service_states(project, env, cwd, deadline, popen_factory, selector_factory, monotonic):
    if not isinstance(project, str) or not _PROJECT.fullmatch(project):
        return None
    raw_ids = _run_probe(
        [
            "docker", "ps", "-a", "-q", "--filter",
            f"label=com.docker.compose.project={project}",
        ], env, cwd, deadline,
        popen_factory, selector_factory, monotonic,
    )
    if raw_ids is None:
        return None
    ids = raw_ids.splitlines()
    if len(ids) > len(SERVICES) or len(ids) != len(set(ids)):
        return None
    if any(not _CONTAINER_ID.fullmatch(container_id) for container_id in ids):
        return None
    observed = {}
    if ids:
        raw_states = _run_probe(
            [
                "docker",
                "inspect",
                "--format",
                _INSPECT_FORMAT,
                *(container_id.decode("ascii") for container_id in ids),
            ],
            env, cwd, deadline, popen_factory, selector_factory, monotonic,
        )
        if raw_states is None:
            return None
        observed = _parse_inspect(raw_states, ids)
        if observed is None:
            return None
    return ",".join(
        (
            f"{service}:{':'.join(observed[service])}"
            if service in observed
            else f"{service}:absent"
        )
        for service in SERVICES
    )


def initial_start_failure_diagnostic(
    stderr,
    project,
    env,
    cwd,
    *,
    deadline_seconds=DIAGNOSTIC_SECONDS,
    popen_factory=None,
    selector_factory=selectors.DefaultSelector,
    monotonic=time.monotonic,
):
    """Build a fixed-shape diagnostic without exposing subprocess output."""
    category = classify_start_failure(stderr).value
    deadline = monotonic() + deadline_seconds
    states = _service_states(
        project, env, cwd, deadline, popen_factory or subprocess.Popen, selector_factory, monotonic
    )
    safe_states = states if states is not None else "unavailable"
    return f"diagnostic_category={category}; service_states={safe_states}"


def require_successful_initial_start(result, project, env, cwd, **diagnostic_options):
    if result.returncode:
        try:
            diagnostic = initial_start_failure_diagnostic(
                result.stderr, project, env, cwd, **diagnostic_options
            )
        except Exception:
            diagnostic = "diagnostic_category=unclassified; service_states=unavailable"
        raise RuntimeError("fixture command failed: docker; " + diagnostic)
    return result


def cleanup_preserving_active_failure(cleanup):
    """Run cleanup, suppressing its failure only while another failure is active."""
    failure_is_active = sys.exc_info()[0] is not None
    try:
        cleanup()
    except Exception:
        if not failure_is_active:
            raise


def cleanup_fixture(compose_exists, compose_cleanup, work_cleanup):
    """Protect the primary failure and always attempt both fixture cleanups."""
    try:
        cleanup_preserving_active_failure(lambda: compose_cleanup() if compose_exists() else None)
    finally:
        cleanup_preserving_active_failure(work_cleanup)
