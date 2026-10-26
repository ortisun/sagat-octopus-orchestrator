"""Destructive recovery drill restricted to the calling disposable Compose fixture."""
import json
import ssl
import time
import urllib.error
import urllib.request
import uuid


def restore(dc, private, address, verify):
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            return None

    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({}), NoRedirect(),
        urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(private / "tls/ca.crt"))),
    )
    init = json.loads((private / "development-unseal.json").read_text())

    def bao(method, path, body=None, expected=200, binary=False):
        headers = {"X-Vault-Token": init["root_token"], "Content-Type": "application/octet-stream" if binary else "application/json"}
        data = body if binary else (None if body is None else json.dumps(body).encode())
        request = urllib.request.Request(address + "/v1/" + path, method=method, headers=headers, data=data)
        try:
            response = opener.open(request, timeout=60)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            assert response.code == expected, f"recovery fixture OpenBao status {response.code} on {method} {path}"
            payload = response.read(64 * 1024 * 1024 + 1)
            assert len(payload) <= 64 * 1024 * 1024, "fixture snapshot exceeds bound"
            return payload if binary else (json.loads(payload) if payload else {})

    # Real prior retirement metadata must survive the backup, including destroyed version 1.
    retired = "sagat/credentials/" + str(uuid.uuid4()) + "/" + str(uuid.uuid4()) + "/" + str(uuid.uuid4())
    bao("POST", "secret/data/" + retired, {"options":{"cas":0}, "data":{"retired":True}})
    bao("POST", "secret/destroy/" + retired, {"versions":[1]}, expected=204)
    dc("stop", "database-runner", "control-plane", "execution-engine")
    # No cleanup job, external writer or other application process exists in this fixture.
    pending = dc("exec", "-T", "postgres", "psql", "-U", "sagat", "-d", "sagat", "-Atc",
                 "SELECT count(*) FROM secret_provider_operations WHERE state='pending'").stdout.strip()
    assert pending == b"0", "settle provider intents before taking the coordinated recovery point"
    dump = dc("exec", "-T", "postgres", "pg_dump", "-U", "sagat", "-d", "sagat", "--format=custom").stdout
    snapshot = bao("GET", "sys/storage/raft/snapshot", binary=True)
    key = (private / "intent/intent-key").read_bytes()
    assert len(key) == 32
    for name, data in [("platform.dump", dump), ("openbao.snap", snapshot), ("intent-key.backup", key)]:
        backup = private / name
        backup.write_bytes(data)
        backup.chmod(0o600)
    credential = dc("exec", "-T", "postgres", "psql", "-U", "sagat", "-d", "sagat", "-Atc",
                    "SELECT 'sagat/credentials/'||tenant_id||'/'||namespace_id||'/'||id FROM secret_provider_operations WHERE state='active' LIMIT 1").stdout.decode().strip()
    assert credential.startswith("sagat/credentials/")
    # Simulate lost metadata and provider values after the saved point. The customer DB stays intact.
    bao("DELETE", "secret/metadata/" + credential, expected=204)
    bao("DELETE", "secret/metadata/" + retired, expected=204)
    bao("GET", "secret/data/" + credential, expected=404)
    dc("exec", "-T", "postgres", "dropdb", "-U", "sagat", "--force", "sagat")
    dc("exec", "-T", "postgres", "createdb", "-U", "sagat", "sagat")
    dc("exec", "-T", "postgres", "pg_restore", "-U", "sagat", "-d", "sagat", "--exit-on-error", data=(private / "platform.dump").read_bytes())
    # Same cluster/key material: normal restore verifies the snapshot seal, no force bypass.
    bao("POST", "sys/storage/raft/snapshot", (private / "openbao.snap").read_bytes(), expected=204, binary=True)
    dc("restart", "openbao")
    deadline = time.monotonic() + 60
    while True:
        try:
            status = bao("GET", "sys/seal-status")
            if status["sealed"]:
                bao("PUT", "sys/unseal", {"key":init["keys_base64"][0]})
            if bao("GET", "sys/health").get("sealed") is False:
                break
        except (OSError, AssertionError):
            if time.monotonic() >= deadline:
                raise RuntimeError("restored OpenBao did not become healthy") from None
            time.sleep(.5)
    (private / "intent/intent-key").write_bytes((private / "intent-key.backup").read_bytes())
    assert (private / "intent/intent-key").read_bytes() == key
    assert bao("GET", "secret/data/" + credential)["data"]["metadata"]["version"] == 1
    assert bao("GET", "secret/metadata/" + retired)["data"]["versions"]["1"]["destroyed"] is True
    bao("POST", "secret/data/" + retired, {"options":{"cas":0}, "data":{"late":True}}, expected=400)
    dc("up", "-d", "control-plane", "execution-engine", timeout=300)
    dc("--profile", "database", "up", "-d", "database-runner", timeout=300)
    verify()
    print("PASS: coordinated PostgreSQL/Raft restore, process restart, retained result, live credential and permanent retirement fence", flush=True)
