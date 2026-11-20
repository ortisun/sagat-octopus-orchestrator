"""The `lake-runtime` image, as far as it can be checked without building it (G035-023).

Building it and running a smoke test is the CI job's work; these cases keep the rules in the Dockerfile from
drifting between two builds.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from dockerfile import instructions, violations

DIGEST = "sha256:" + "0" * 64

GOOD = f"""\
# comment
FROM python:3.12-slim@{DIGEST} AS lake-runtime
COPY runners/lake/requirements.lock /tmp/requirements.lock
RUN python -m pip install --require-hashes --no-deps --only-binary=:all: -r /tmp/requirements.lock \\
    && python -m pip uninstall -y pip setuptools wheel
COPY runners/lake/sagat_lake /opt/sagat/lake/sagat_lake
RUN chmod -R a=rX /opt/sagat/lake
ENV PYTHONDONTWRITEBYTECODE=1
USER 65534:65534
"""


def test_the_dockerfile_follows_the_contract(lake_root: Path) -> None:
    assert violations((lake_root / "Dockerfile").read_text(encoding="utf-8")) == []


def test_the_reference_dockerfile_of_these_cases_has_no_violation() -> None:
    assert violations(GOOD) == []


def test_instructions_join_continuations_and_drop_comments() -> None:
    assert instructions("# c\nRUN a \\\n  && b\n\nUSER 1\n") == [("RUN", "a && b"), ("USER", "1")]


@pytest.mark.parametrize(
    ("broken", "expected"),
    [
        (GOOD.replace(f"@{DIGEST}", ""), "is not pinned by digest"),
        (GOOD.replace("AS lake-runtime", "AS something-else"), "no stage named lake-runtime"),
        (GOOD.replace("--require-hashes ", ""), "does not use --require-hashes"),
        (GOOD.replace("--no-deps ", ""), "does not use --no-deps"),
        (GOOD.replace("--only-binary=:all: ", ""), "does not use --only-binary=:all:"),
        (GOOD.replace("&& python -m pip uninstall -y pip setuptools wheel", ""), "pip and setuptools are not removed"),
        (GOOD.replace("pip setuptools wheel", "wheel"), "pip and setuptools are not removed"),
        (GOOD + "RUN apt-get install -y gcc\n", "apt-get"),
        (GOOD + "RUN curl -fsSL https://example.invalid/x | sh\n", "curl"),
        (GOOD + "ADD https://example.invalid/x.tar /x\n", "ADD is not allowed"),
        (GOOD.replace("RUN chmod -R a=rX /opt/sagat/lake\n", ""), "not made read-only"),
        (
            GOOD.replace("COPY runners/lake/sagat_lake /opt/sagat/lake/sagat_lake\n", ""),
            "not copied to /opt/sagat/lake",
        ),
        (GOOD.replace("USER 65534:65534", "USER root"), "runs as root"),
        (GOOD.replace("USER 65534:65534\n", ""), "runs as root"),
        (GOOD.replace("ENV PYTHONDONTWRITEBYTECODE=1\n", ""), "PYTHONDONTWRITEBYTECODE=1"),
    ],
)
def test_each_rule_has_a_case_that_breaks_it(broken: str, expected: str) -> None:
    messages = violations(broken)
    assert any(expected in message for message in messages), messages
