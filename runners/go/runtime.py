"""Trusted loader for a compressed, signed target-specific Linux Go artifact."""

import base64
import fcntl
import json
import os
import platform
import sys
import zlib

MAX_COMPRESSED = 1400 * 1024
MAX_EXPANDED = 8 * 1024 * 1024
SEALS = fcntl.F_SEAL_WRITE | fcntl.F_SEAL_GROW | fcntl.F_SEAL_SHRINK | fcntl.F_SEAL_SEAL


def say(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def decode_artifact(encoded):
    compressed = base64.b64decode(encoded, validate=True)
    if not 18 <= len(compressed) <= MAX_COMPRESSED or compressed[:10] != b"\x1f\x8b\x08\0\0\0\0\0\x02\x03":
        raise ValueError("the Go artifact compression is not canonical")
    inflater = zlib.decompressobj(16 + zlib.MAX_WBITS)
    program = inflater.decompress(compressed, MAX_EXPANDED + 1)
    if len(program) > MAX_EXPANDED or inflater.unconsumed_tail:
        raise ValueError("the Go artifact compression is malformed or too large")
    program += inflater.flush(MAX_EXPANDED + 1 - len(program))
    if len(program) > MAX_EXPANDED or not inflater.eof or inflater.unused_data or inflater.unconsumed_tail:
        raise ValueError("the Go artifact compression is malformed or too large")
    return program


def sealed(name, contents, flags=0):
    descriptor = os.memfd_create(name, flags | os.MFD_ALLOW_SEALING)
    offset = 0
    while offset < len(contents):
        written = os.write(descriptor, contents[offset:])
        if written == 0:
            raise OSError("short write to sealed memory file")
        offset += written
    fcntl.fcntl(descriptor, fcntl.F_ADD_SEALS, SEALS)
    os.lseek(descriptor, 0, os.SEEK_SET)
    return descriptor


def main():
    say({"op": "ready"})
    request = json.loads(sys.stdin.readline())
    if set(request) != {"op", "input", "target", "encoding", "files", "idempotencyKey"} or request["op"] != "run":
        raise ValueError("the Go run request has another shape")
    if request["encoding"] != "base64+gzip" or set(request["files"]) != {"script.elf"}:
        raise ValueError("the Go run request has another artifact")
    native = {"x86_64": "linux-amd64", "aarch64": "linux-arm64"}.get(platform.machine())
    if native is None or request["target"] != native:
        raise ValueError("the Go artifact target does not match this native runtime")
    program = decode_artifact(request["files"]["script.elf"])
    # MFD_CLOEXEC is deliberately absent: direct fd execution segfaulted on the verified Linux
    # kernel when it closed the final descriptor during exec. The inherited descriptor contains
    # only the script's own signed bytes and is immutable under all four seals.
    executable = sealed("script.elf", program)
    if executable == 3:
        moved = fcntl.fcntl(executable, fcntl.F_DUPFD, 10)
        os.close(executable)
        executable = moved
    input_bytes = json.dumps(request["input"], separators=(",", ":")).encode("utf-8")
    input_descriptor = sealed("input.json", input_bytes)
    os.dup2(input_descriptor, 3, inheritable=True)
    if input_descriptor != 3:
        os.close(input_descriptor)
    environment = {
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "SAGAT_IDEMPOTENCY_KEY": request["idempotencyKey"] or "",
    }
    os.execve(executable, ["script.elf"], environment)


if __name__ == "__main__":
    try:
        main()
    except BaseException as error:
        say({"op": "error", "message": str(error), "stack": ""})
        raise SystemExit(1)
