# The supervisor of a Python script inside its sandbox. It reads the input from the platform, calls
# `run(input, octopus)` of the script and writes the result back. Whatever the script prints goes to
# standard error (the log); standard output is the platform's channel. The script has no network and
# no secrets: `octopus.fetch` and `octopus.secret` ask the platform, which may refuse.
import importlib.util
import json
import os
import re
import sys
import traceback

_channel = sys.stdout
sys.stdout = sys.stderr


def _write(message):
    _channel.write(json.dumps(message, separators=(",", ":")) + "\n")
    _channel.flush()


_next_id = 0


def _call(message):
    global _next_id
    _next_id += 1
    _write({**message, "id": _next_id})
    while True:
        line = sys.stdin.readline()
        if not line:
            raise RuntimeError("the platform closed the channel")
        reply = json.loads(line)
        if reply.get("id") == _next_id:
            if reply.get("ok"):
                return reply
            raise PermissionError(reply.get("error") or "refused")


class _Response:
    def __init__(self, reply):
        self.status = reply.get("status")
        self.headers = reply.get("headers") or {}
        self._text = reply.get("body") or ""

    def text(self):
        return self._text

    def json(self):
        return json.loads(self._text)


class _Octopus:
    def fetch(self, url, method="GET", headers=None, body=None):
        reply = _call(
            {
                "op": "fetch",
                "url": str(url),
                "method": method,
                "headers": headers or {},
                "body": None if body is None else str(body),
            }
        )
        return _Response(reply)

    def secret(self, name):
        return _call({"op": "secret", "name": str(name)}).get("value")

    def log(self, *args):
        print(*args, file=sys.stderr)


def _main():
    _write({"op": "ready"})
    first = sys.stdin.readline()
    code = 0
    try:
        message = json.loads(first)
        # The code arrives on the channel and is written where the script can read it but not run it
        # as a program: the scratch area is mounted without the execute bit.
        os.makedirs("/tmp/script", exist_ok=True)
        for name, text in (message.get("files") or {}).items():
            if re.fullmatch(r"[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}", name):
                with open("/tmp/script/" + name, "w", encoding="utf-8") as handle:
                    handle.write(text)
        spec = importlib.util.spec_from_file_location("script", "/tmp/script/main.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        entry = getattr(module, "run", None)
        if not callable(entry):
            raise RuntimeError("the script must define run(input, octopus)")
        value = entry(message["input"], _Octopus())
        _write({"op": "result", "value": value})
    except BaseException as error:  # noqa: BLE001 - everything the script does ends here
        code = 1
        _write({"op": "error", "message": str(error), "stack": traceback.format_exc()})
    sys.stderr.flush()
    os._exit(code)


_main()
