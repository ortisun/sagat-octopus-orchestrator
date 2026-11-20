"""The rules of the `lake-runtime` image as a function over the Dockerfile text.

contracts/lake-runtime.md ("Image") and G035-023 fix what the image may contain. Prose does not stop a later edit
from adding a compiler or an unpinned base, so the rules are checked here, and each rule has a case that breaks it.
"""

from __future__ import annotations

import re

STAGE = "lake-runtime"
SOURCE_DIRECTORY = "/opt/sagat/lake"
_PINNED = re.compile(r"@sha256:[0-9a-f]{64}$")
_FORBIDDEN_IN_RUN = (
    "apt-get",
    "apt ",
    "apk ",
    "yum ",
    "dnf ",
    "gcc",
    "g++",
    "build-essential",
    "make ",
    "rustc",
    "cargo",
    "curl",
    "wget",
)


def instructions(text: str) -> list[tuple[str, str]]:
    """(keyword, arguments) of each instruction: comments dropped, `\\` continuations joined."""
    joined: list[str] = []
    pending = ""
    for raw in text.splitlines():
        line = raw.strip()
        if not pending and (not line or line.startswith("#")):
            continue
        if line.startswith("#"):
            continue
        if line.endswith("\\"):
            pending += line[:-1].strip() + " "
            continue
        joined.append(pending + line)
        pending = ""
    if pending:
        joined.append(pending.strip())
    result: list[tuple[str, str]] = []
    for line in joined:
        keyword, _, arguments = line.partition(" ")
        result.append((keyword.upper(), arguments.strip()))
    return result


def violations(text: str) -> list[str]:
    """What the Dockerfile does that the contract does not allow; empty when it follows the contract."""
    found: list[str] = []
    steps = instructions(text)

    stages: list[str] = []
    for keyword, arguments in steps:
        if keyword != "FROM":
            continue
        words = [word for word in arguments.split() if not word.startswith("--")]
        base = words[0]
        if base not in stages and not _PINNED.search(base):
            found.append(f"the base image {base} is not pinned by digest")
        if len(words) >= 3 and words[1].lower() == "as":
            stages.append(words[2])
    if STAGE not in stages:
        found.append(f"there is no stage named {STAGE}")
        return found

    in_stage = False
    stage_steps: list[tuple[str, str]] = []
    for keyword, arguments in steps:
        if keyword == "FROM":
            in_stage = arguments.split()[-1] == STAGE
            continue
        if in_stage:
            stage_steps.append((keyword, arguments))

    runs = [arguments for keyword, arguments in stage_steps if keyword == "RUN"]
    installs = [run for run in runs if "pip install" in run]
    if not installs:
        found.append("the stage installs nothing with pip")
    for install in installs:
        for flag in ("--require-hashes", "--no-deps", "--only-binary=:all:", "requirements.lock"):
            if flag not in install:
                found.append(f"the pip install does not use {flag}")
    uninstall = [run for run in runs if "pip uninstall" in run]
    if not uninstall or not all(package in uninstall[-1] for package in ("pip", "setuptools")):
        found.append("pip and setuptools are not removed after the installation")
    for run in runs:
        for token in _FORBIDDEN_IN_RUN:
            if token in run:
                found.append(f"a RUN uses {token.strip()}, which the image must not contain or fetch with")
    for keyword, _ in stage_steps:
        if keyword == "ADD":
            found.append("ADD is not allowed; COPY the reviewed files")
    copies = [arguments for keyword, arguments in stage_steps if keyword == "COPY" and SOURCE_DIRECTORY in arguments]
    if not copies:
        found.append(f"the host program is not copied to {SOURCE_DIRECTORY}")
    elif not any(f"chmod -R a=rX {SOURCE_DIRECTORY}" in run for run in runs):
        found.append(f"{SOURCE_DIRECTORY} is not made read-only after the copy")

    users = [arguments for keyword, arguments in stage_steps if keyword == "USER"]
    if not users or users[-1].split(":")[0] in ("root", "0"):
        found.append("the image runs as root")
    environment = " ".join(arguments for keyword, arguments in stage_steps if keyword == "ENV")
    if "PYTHONDONTWRITEBYTECODE=1" not in environment:
        found.append("PYTHONDONTWRITEBYTECODE=1 is not set, and the root filesystem is read-only")
    return found
