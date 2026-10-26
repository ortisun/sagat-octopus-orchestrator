"""Safe diagnostics for the database Compose fixture."""
import ast
import inspect
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import uuid


ROOT = pathlib.Path(__file__).resolve().parents[2]
HELPERS = ROOT / "tests/e2e/database-connections"
COMPOSE_SCRIPT = HELPERS / "compose.py"
sys.path.insert(0, str(HELPERS))

import compose_diagnostics as diagnostics


CANARY = "credential-canary-do-not-print"
REAL_CHILD = "SAGAT_REAL_COMPOSE_DIAGNOSTIC_CHILD"
REAL_FIXTURE = "SAGAT_REAL_COMPOSE_DIAGNOSTIC_FIXTURE"
REAL_PROJECT = "SAGAT_REAL_COMPOSE_DIAGNOSTIC_PROJECT"
PINNED_IMAGE = (
    "docker.io/openbao/openbao:2.7.1@"
    "sha256:6d2b93856e3fcf7b18ad855a0b51eaba474dc8b79cf554379ea32034797d2acf"
)


def completed(argv, code=0, stdout=b"", stderr=b""):
    return subprocess.CompletedProcess(argv, code, stdout, stderr)


class PipeProcess:
    """A selectable, already-completed process double."""

    def __init__(self, result):
        read_fd, write_fd = os.pipe()
        os.write(write_fd, result.stdout)
        os.close(write_fd)
        self.stdout = os.fdopen(read_fd, "rb", buffering=0)
        self.pid = 99_999_999
        self.returncode = result.returncode

    def wait(self, timeout=None):
        return self.returncode

    def kill(self):
        self.returncode = -signal.SIGKILL


class MutableClock:
    def __init__(self, now):
        self.now = now

    def __call__(self):
        return self.now


class LatePipeProcess(PipeProcess):
    def __init__(self, result, clock, completed_at):
        super().__init__(result)
        self.clock = clock
        self.completed_at = completed_at

    def wait(self, timeout=None):
        self.clock.now = self.completed_at
        return self.returncode


class PopenQueue:
    def __init__(self, *results):
        self.results = list(results)
        self.calls = []

    def __call__(self, argv, **kwargs):
        self.calls.append((argv, kwargs))
        result = self.results.pop(0)
        if isinstance(result, Exception):
            raise result
        return PipeProcess(result)


def supervise_owned_worker(
    child, cleanup_project, cleanup_work, validate_worker=lambda returncode: None,
    kill_group=os.killpg,
):
    """Supervise the controlled Compose worker and its parent-owned resources."""
    worker_reaped = False

    def stop_worker():
        try:
            kill_group(child.pid, signal.SIGKILL)
        except (OSError, ProcessLookupError):
            pass
        try:
            child.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            pass

    try:
        returncode = child.wait(timeout=75)
        worker_reaped = True
        validate_worker(returncode)
        return returncode
    finally:
        try:
            if not worker_reaped:
                diagnostics.cleanup_preserving_active_failure(stop_worker)
        finally:
            try:
                diagnostics.cleanup_preserving_active_failure(cleanup_project)
            finally:
                diagnostics.cleanup_preserving_active_failure(cleanup_work)


class SafeComposeDiagnostics(unittest.TestCase):
    def test_successful_initial_start_does_not_probe(self):
        result = completed(["docker", "compose", "up"])
        returned = diagnostics.require_successful_initial_start(
            result,
            "owned-project",
            {},
            ROOT,
            popen_factory=lambda argv, **kwargs: self.fail("success must not run diagnostics"),
        )
        self.assertIs(returned, result)

    def test_initial_start_failure_carries_safe_evidence(self):
        popen = PopenQueue(
            completed([], stdout=("a" * 64 + "\n").encode()),
            completed([], stdout=b'"migrate"\t"exited"\t42\t"none"\n'),
        )

        result = completed(
            ["docker", "compose", "up"],
            code=1,
            stdout=CANARY.encode(),
            stderr=f"dependency failed to start {CANARY}".encode(),
        )
        with self.assertRaises(RuntimeError) as caught:
            diagnostics.require_successful_initial_start(
                result,
                "owned-project",
                {},
                ROOT,
                popen_factory=popen,
            )

        message = str(caught.exception)
        self.assertTrue(
            "diagnostic_category=dependency_failed" in message,
            "startup failure omitted its safe category",
        )
        self.assertTrue(
            "migrate:exited:42:none" in message,
            "startup failure omitted its safe service state",
        )
        self.assertFalse(
            CANARY in message,
            "startup failure exposed captured diagnostic input",
        )
        self.assertEqual(len(popen.calls), 2)

    def test_internal_diagnostic_failure_uses_a_static_fallback(self):
        result = completed(
            ["docker", "compose", "up"], code=1, stderr=CANARY.encode()
        )
        with self.assertRaises(RuntimeError) as caught:
            diagnostics.require_successful_initial_start(
                result,
                "owned-project",
                {"SAGAT_PRIVATE_VALUE": CANARY},
                ROOT / CANARY,
                monotonic=lambda: (_ for _ in ()).throw(RuntimeError(CANARY)),
            )
        self.assertTrue(
            str(caught.exception)
            == "fixture command failed: docker; diagnostic_category=unclassified; "
            "service_states=unavailable",
            "internal diagnostic failure did not use the static fallback",
        )

    def test_failure_reports_only_classification_and_allowlisted_state(self):
        project = f"owned-{CANARY}"
        popen = PopenQueue(
            completed([], stdout=("a" * 64 + "\n").encode(), stderr=CANARY.encode()),
            completed([], stdout=b'"migrate"\t"exited"\t42\t"none"\n', stderr=CANARY.encode()),
        )

        message = diagnostics.initial_start_failure_diagnostic(
            f"dependency failed to start {CANARY}".encode(),
            project,
            {"SAGAT_PRIVATE_VALUE": CANARY},
            ROOT / CANARY,
            popen_factory=popen,
        )

        self.assertTrue(
            "diagnostic_category=dependency_failed" in message,
            "diagnostic omitted the closed category",
        )
        self.assertTrue(
            "migrate:exited:42:none" in message,
            "diagnostic omitted the allowlisted service state",
        )
        self.assertTrue(
            "openbao:absent" in message,
            "diagnostic omitted an absent service marker",
        )
        self.assertFalse(CANARY in message, "diagnostic exposed a forbidden input")
        self.assertTrue(
            popen.calls[0][0]
            == [
                "docker", "ps", "-a", "-q", "--filter",
                f"label=com.docker.compose.project={project}",
            ],
            "diagnostic did not use the exact project-label filter",
        )
        self.assertTrue(
            popen.calls[1][0][:3] == ["docker", "inspect", "--format"],
            "diagnostic did not inspect only validated container identifiers",
        )
        self.assertNotIn(".Error", popen.calls[1][0][3])
        self.assertNotIn(".Log", popen.calls[1][0][3])
        self.assertEqual(popen.calls[0][1]["stderr"], subprocess.DEVNULL)
        self.assertTrue(
            "start_new_session" not in popen.calls[0][1],
            "diagnostic probe escaped the supervised worker process group",
        )

    def test_ambiguous_unknown_and_oversized_errors_are_unclassified(self):
        ambiguous = b"port is already allocated; no space left on device"
        self.assertEqual(diagnostics.classify_start_failure(ambiguous).value, "unclassified")
        self.assertEqual(
            diagnostics.classify_start_failure(f"unexpected {CANARY}".encode()).value,
            "unclassified",
        )
        oversized = b"port is already allocated" + b"x" * diagnostics.MAX_CAPTURE_BYTES
        self.assertEqual(diagnostics.classify_start_failure(oversized).value, "unclassified")

    def test_malformed_ids_and_probe_exceptions_emit_only_unavailable(self):
        valid = "d" * 64
        cases = [
            ("invalid_id", completed([], stdout=f"{CANARY}\n".encode())),
            ("too_many_ids", completed([], stdout=((valid + "\n") * 8).encode())),
            ("duplicate_id", completed([], stdout=(valid + "\n" + valid + "\n").encode())),
            ("spawn_error", OSError(CANARY)),
        ]
        for case_id, result in cases:
            with self.subTest(case=case_id):
                message = diagnostics.initial_start_failure_diagnostic(
                    CANARY.encode(),
                    "owned-project",
                    {},
                    ROOT,
                    popen_factory=PopenQueue(result),
                )
                self.assertTrue(
                    "service_states=unavailable" in message,
                    "invalid probe output did not become unavailable",
                )
                self.assertFalse(
                    CANARY in message,
                    "diagnostic included forbidden probe input",
                )

    def test_malformed_inspect_fields_never_escape(self):
        bad_lines = [
            ("unknown_service", f'"{CANARY}"\t"running"\t0\t"healthy"\n'),
            ("unknown_state", f'"migrate"\t"{CANARY}"\t0\t"healthy"\n'),
            ("unknown_health", f'"migrate"\t"running"\t0\t"{CANARY}"\n'),
            ("malformed", CANARY + "\n"),
        ]
        for case_id, bad in bad_lines:
            popen = PopenQueue(
                completed([], stdout=("b" * 64 + "\n").encode()),
                completed([], stdout=bad.encode(), stderr=CANARY.encode()),
            )
            with self.subTest(case=case_id):
                message = diagnostics.initial_start_failure_diagnostic(
                    b"unknown",
                    "owned-project",
                    {},
                    ROOT,
                    popen_factory=popen,
                )
                self.assertFalse(
                    CANARY in message,
                    "diagnostic included forbidden inspect input",
                )
                self.assertTrue(
                    "service_states=unavailable" in message,
                    "malformed inspect state did not invalidate the snapshot",
                )

    def test_invalid_exit_code_invalidates_the_whole_snapshot(self):
        cases = (
            ("negative", "-1"),
            ("over_255", "256"),
            ("leading_plus", "+42"),
            ("underscore", "1_0"),
            ("negative_zero", "-0"),
            ("leading_zero", "042"),
        )
        for case_id, exit_code in cases:
            popen = PopenQueue(
                completed([], stdout=("e" * 64 + "\n").encode()),
                completed(
                    [],
                    stdout=f'"migrate"\t"exited"\t{exit_code}\t"none"\n'.encode(),
                ),
            )
            with self.subTest(case=case_id):
                message = diagnostics.initial_start_failure_diagnostic(
                    b"unknown",
                    "owned-project",
                    {},
                    ROOT,
                    popen_factory=popen,
                )
                self.assertTrue(
                    "service_states=unavailable" in message,
                    "invalid exit code did not invalidate the snapshot",
                )

    def test_probe_overflow_is_killed_before_the_deadline(self):
        with tempfile.TemporaryDirectory(prefix="sagat-diagnostic-overflow-") as directory:
            pid_file = pathlib.Path(directory) / "pid"
            program = (
                "import os,pathlib,signal;"
                f"pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()));"
                f"os.write(1,b'x'*({diagnostics.MAX_CAPTURE_BYTES}+1));"
                "signal.pause()"
            )
            started = time.monotonic()
            message = diagnostics.initial_start_failure_diagnostic(
                b"unknown",
                "owned-project",
                {},
                ROOT,
                deadline_seconds=2.0,
                popen_factory=lambda argv, **kwargs: subprocess.Popen(
                    [sys.executable, "-c", program], **kwargs
                ),
            )
            elapsed = time.monotonic() - started
            self.assertTrue(elapsed < 1.0, "overflow was not stopped at the capture bound")
            self.assertTrue(
                "service_states=unavailable" in message,
                "overflow did not make the snapshot unavailable",
            )
            pid = int(pid_file.read_text())
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, signal.SIGCONT)

    def test_stdout_closed_process_is_stopped_before_the_reap_reserve(self):
        with tempfile.TemporaryDirectory(prefix="sagat-diagnostic-eof-") as directory:
            pid_file = pathlib.Path(directory) / "pid"
            program = (
                "import os,pathlib,signal;"
                f"pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()));"
                "os.close(1);signal.pause()"
            )
            started = time.monotonic()
            message = diagnostics.initial_start_failure_diagnostic(
                b"unknown",
                "owned-project",
                {},
                ROOT,
                deadline_seconds=1.0,
                popen_factory=lambda argv, **kwargs: subprocess.Popen(
                    [sys.executable, "-c", program], **kwargs
                ),
            )
            elapsed = time.monotonic() - started
            self.assertTrue(
                elapsed < 0.9,
                "stdout-closed probe consumed its reserved reap budget",
            )
            self.assertTrue(
                "service_states=unavailable" in message,
                "stdout-closed running probe did not become unavailable",
            )
            pid = int(pid_file.read_text())
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, signal.SIGCONT)

    def test_completion_after_overall_deadline_is_rejected(self):
        clock = MutableClock(10.0)
        process = LatePipeProcess(completed([], stdout=b""), clock, 16.0)
        result = diagnostics._run_probe(
            ["docker", "ps"],
            {},
            ROOT,
            15.0,
            lambda argv, **kwargs: process,
            diagnostics.selectors.DefaultSelector,
            clock,
        )
        self.assertTrue(result is None, "late probe completion was accepted")

    def test_each_known_error_class_is_static(self):
        cases = {
            b"Cannot connect to the Docker daemon": "daemon_unavailable",
            b"port is already allocated": "host_port_unavailable",
            b"Pool overlaps with other one on this address space": "network_subnet_conflict",
            b"manifest unknown": "image_unavailable",
            b"dependency failed to complete successfully": "dependency_failed",
            b"service did not start: didn't complete successfully": "dependency_failed",
            b"OCI runtime create failed": "container_runtime_failed",
            b"invalid mount config": "mount_failed",
            b"no space left on device": "host_resource_exhausted",
        }
        for raw, expected in cases.items():
            with self.subTest(expected=expected):
                self.assertEqual(diagnostics.classify_start_failure(raw).value, expected)

    def test_default_deadline_is_five_seconds_and_expired_budget_does_not_spawn(self):
        default = inspect.signature(
            diagnostics.initial_start_failure_diagnostic
        ).parameters["deadline_seconds"].default
        self.assertTrue(
            diagnostics.DIAGNOSTIC_SECONDS == 5.0 and default == 5.0,
            "diagnostic default deadline changed",
        )
        clock = iter((10.0, 15.0))
        spawn_calls = []

        def unexpected_spawn(argv, **kwargs):
            spawn_calls.append(True)
            raise OSError(CANARY)

        message = diagnostics.initial_start_failure_diagnostic(
            b"unknown",
            "owned-project",
            {},
            ROOT,
            popen_factory=unexpected_spawn,
            monotonic=lambda: next(clock),
        )
        self.assertTrue(
            len(spawn_calls) == 0,
            "expired diagnostic budget started a process",
        )
        self.assertTrue(
            "service_states=unavailable" in message,
            "expired budget did not produce unavailable state",
        )

    def test_cleanup_failure_does_not_mask_an_active_failure(self):
        def fail_cleanup():
            raise RuntimeError(f"cleanup {CANARY}")

        caught = None
        try:
            try:
                raise RuntimeError("startup failure")
            finally:
                diagnostics.cleanup_preserving_active_failure(fail_cleanup)
        except RuntimeError as error:
            caught = error
        self.assertTrue(caught is not None, "active startup failure was lost")
        self.assertTrue(
            str(caught) == "startup failure",
            "cleanup failure masked the active startup failure",
        )

    def test_real_fixture_finally_uses_protected_cleanup_caller(self):
        tree = ast.parse(COMPOSE_SCRIPT.read_text())
        finally_calls = [
            node
            for block in ast.walk(tree)
            if isinstance(block, ast.Try)
            for node in ast.walk(ast.Module(body=block.finalbody, type_ignores=[]))
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Name)
            and node.func.id == "cleanup_fixture"
        ]
        self.assertTrue(
            len(finally_calls) == 1,
            "fixture finally does not use the protected cleanup caller",
        )
        work_cleanup_calls = []
        compose_cleanup_calls = []

        def fail_guard():
            raise OSError(CANARY)

        caught = None
        try:
            try:
                raise RuntimeError("startup failure")
            finally:
                diagnostics.cleanup_fixture(
                    fail_guard,
                    lambda: compose_cleanup_calls.append(True),
                    lambda: work_cleanup_calls.append(True),
                )
        except RuntimeError as error:
            caught = error
        self.assertTrue(caught is not None, "startup failure was lost")
        self.assertTrue(
            str(caught) == "startup failure",
            "raising cleanup guard masked the startup failure",
        )
        self.assertTrue(
            len(compose_cleanup_calls) == 0,
            "compose cleanup ran after its guard failed",
        )
        self.assertTrue(
            len(work_cleanup_calls) == 1,
            "work cleanup was not attempted after guard failure",
        )

    def test_cleanup_failure_is_not_hidden_without_an_active_failure(self):
        with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
            diagnostics.cleanup_preserving_active_failure(
                lambda: (_ for _ in ()).throw(RuntimeError("cleanup failed"))
            )

    def test_worker_interrupt_kills_and_reaps_before_project_cleanup(self):
        events = []

        class InterruptedWorker:
            pid = 4242

            def __init__(self):
                self.waits = 0

            def wait(self, timeout):
                self.waits += 1
                events.append("wait" if self.waits == 1 else "reap")
                if self.waits == 1:
                    raise KeyboardInterrupt()
                return -signal.SIGKILL

        caught = None
        try:
            supervise_owned_worker(
                InterruptedWorker(),
                lambda: events.append("down"),
                lambda: events.append("work"),
                kill_group=lambda pid, sig: events.append("kill"),
            )
        except KeyboardInterrupt as error:
            caught = error
        self.assertTrue(caught is not None, "worker interruption was lost")
        self.assertTrue(
            events == ["wait", "kill", "reap", "down", "work"],
            "worker was not reaped before owned project cleanup",
        )

    def test_temp_cleanup_failure_preserves_worker_assertion(self):
        events = []

        class FailedWorker:
            pid = 4343

            def __init__(self):
                self.waits = 0

            def wait(self, timeout):
                self.waits += 1
                events.append("wait" if self.waits == 1 else "reap")
                if self.waits == 1:
                    raise AssertionError("worker assertion")
                return -signal.SIGKILL

        caught = None
        try:
            supervise_owned_worker(
                FailedWorker(),
                lambda: events.append("down"),
                lambda: (_ for _ in ()).throw(OSError(CANARY)),
                kill_group=lambda pid, sig: events.append("kill"),
            )
        except BaseException as error:
            caught = error
        self.assertTrue(
            isinstance(caught, AssertionError),
            "temporary-directory cleanup masked the worker assertion",
        )
        self.assertTrue(
            str(caught) == "worker assertion",
            "temporary-directory cleanup replaced the worker assertion",
        )
        self.assertTrue(
            events == ["wait", "kill", "reap", "down"],
            "owned resources were not cleaned in order",
        )

    @unittest.skipUnless(
        os.environ.get("SAGAT_RUN_REAL_COMPOSE_DIAGNOSTIC") == "1",
        "set SAGAT_RUN_REAL_COMPOSE_DIAGNOSTIC=1 for the controlled Docker failure",
    )
    def test_real_compose_dependency_failure_is_safe_and_project_scoped(self):
        if os.environ.get(REAL_CHILD) == "1":
            project = os.environ.get(REAL_PROJECT)
            fixture_value = os.environ.get(REAL_FIXTURE)
            if not project or not fixture_value:
                raise AssertionError("controlled Compose child ownership is missing")
            compose = ["docker", "compose", "-p", project, "-f", fixture_value]
            result = subprocess.run(
                compose + ["up", "-d", "--pull", "never", "nats"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=20,
            )
            self.assertNotEqual(result.returncode, 0)
            message = diagnostics.initial_start_failure_diagnostic(
                result.stderr, project, os.environ, ROOT
            )
            self.assertTrue(
                "diagnostic_category=dependency_failed" in message,
                "real diagnostic omitted its safe category",
            )
            self.assertTrue(
                "migrate:exited:42:none" in message,
                "real diagnostic omitted the failed dependency state",
            )
            self.assertFalse(
                "sagat-compose-diagnostic" in message,
                "real diagnostic exposed its project name",
            )
            return

        try:
            image = subprocess.run(
                ["docker", "image", "inspect", PINNED_IMAGE],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=10,
                check=False,
            )
        except (OSError, subprocess.SubprocessError):
            raise AssertionError(
                "controlled diagnostic image prerequisite unavailable"
            ) from None
        self.assertTrue(
            image.returncode == 0,
            "controlled diagnostic requires the repository's pinned OpenBao image",
        )
        project = "sagat-compose-diagnostic-" + uuid.uuid4().hex[:8]
        work = tempfile.TemporaryDirectory(prefix="sagat-compose-diagnostic-")
        fixture = pathlib.Path(work.name) / "compose.yaml"
        try:
            fixture.write_text(
                "services:\n"
                "  migrate:\n"
                f"    image: {PINNED_IMAGE}\n"
                "    entrypoint: [sh, -ec]\n"
                "    command: ['exit 42']\n"
                "  nats:\n"
                f"    image: {PINNED_IMAGE}\n"
                "    entrypoint: [sh, -ec]\n"
                "    command: ['exit 0']\n"
                "    depends_on:\n"
                "      migrate: {condition: service_completed_successfully}\n"
            )
        except BaseException:
            diagnostics.cleanup_preserving_active_failure(work.cleanup)
            raise
        compose = ["docker", "compose", "-p", project, "-f", str(fixture)]
        environment = dict(os.environ)
        environment[REAL_CHILD] = "1"
        environment[REAL_PROJECT] = project
        environment[REAL_FIXTURE] = str(fixture)
        timed_out = False
        supervisor_available = True
        cleanup_ok = False
        containers_gone = False
        networks_gone = False

        def cleanup_project():
            nonlocal cleanup_ok, containers_gone, networks_gone
            try:
                down = subprocess.run(
                    compose + ["down", "-v", "--remove-orphans"],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=20,
                    check=False,
                )
                cleanup_ok = down.returncode == 0
                remaining = subprocess.run(
                    [
                        "docker", "ps", "-a", "-q", "--filter",
                        f"label=com.docker.compose.project={project}",
                    ],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    timeout=5,
                    check=False,
                )
                containers_gone = remaining.returncode == 0 and not remaining.stdout
                networks = subprocess.run(
                    [
                        "docker", "network", "ls", "-q", "--filter",
                        f"label=com.docker.compose.project={project}",
                    ],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    timeout=5,
                    check=False,
                )
                networks_gone = networks.returncode == 0 and not networks.stdout
            except (OSError, subprocess.SubprocessError):
                pass

        def require_green_worker(returncode):
            if returncode:
                raise AssertionError("controlled Compose diagnostic child failed")

        try:
            child = subprocess.Popen(
                [
                    sys.executable,
                    "-m",
                    "unittest",
                    "scripts.tests.test_compose_diagnostics."
                    "SafeComposeDiagnostics."
                    "test_real_compose_dependency_failure_is_safe_and_project_scoped",
                ],
                cwd=ROOT,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            try:
                supervise_owned_worker(
                    child, cleanup_project, work.cleanup,
                    validate_worker=require_green_worker,
                )
            except subprocess.TimeoutExpired:
                timed_out = True
        except OSError:
            supervisor_available = False
            diagnostics.cleanup_fixture(lambda: True, cleanup_project, work.cleanup)
        except BaseException:
            diagnostics.cleanup_fixture(lambda: True, cleanup_project, work.cleanup)
            raise
        self.assertTrue(supervisor_available, "controlled supervisor unavailable")
        self.assertFalse(timed_out, "controlled Compose diagnostic exceeded its bound")
        self.assertTrue(cleanup_ok, "controlled project cleanup failed")
        self.assertTrue(
            containers_gone,
            "controlled fixture left owned containers behind",
        )
        self.assertTrue(
            networks_gone,
            "controlled fixture left owned networks behind",
        )


if __name__ == "__main__":
    unittest.main()
