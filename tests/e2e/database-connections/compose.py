#!/usr/bin/env python3
"""Exercise the real built images, TLS OpenBao/PostgreSQL and isolated Docker runner.

Requires images built from this checkout (control-plane, execution-engine, database-runner,
dev-idp). Creates/removes only its own Compose project and private fixture directory.
"""
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from recovery import restore

ROOT = pathlib.Path(__file__).resolve().parents[3]
project = "sagat-db017-proof-" + uuid.uuid4().hex[:8]
work = tempfile.TemporaryDirectory(prefix="sagat-db017-compose-")
private = pathlib.Path(work.name) / "private"
env = dict(os.environ)
# Use unique published ports; no existing user's development stack is stopped or reused.
for name in ("PG", "NATS", "NATS_MONITOR", "IDP", "API", "GRPC", "BAO"):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        env["SAGAT_" + name + "_PORT"] = str(listener.getsockname()[1])
env["SAGAT_ENV"] = "dev"
env["SAGAT_OIDC_ISSUER"] = "http://dev-idp:9100/"
compose = ["docker", "compose", "-p", project, "-f", str(ROOT / "infra/docker/compose.yaml"), "-f", str(ROOT / "infra/docker/compose.database.yaml"), "--env-file", str(ROOT / "infra/docker/.env.example"), "--env-file", str(private / "compose.env")]

# A command that pulls or starts containers gets this much on a cold runner; anything else stays at 60 s.
# A timeout here is a failure that names the command, never a wait without end.
SLOW = 300

def run(argv, data=None, check=True, timeout=60):
    result = subprocess.run(argv, input=data, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, cwd=ROOT)
    if check and result.returncode:
        raise RuntimeError("fixture command failed: " + argv[0])
    return result

def dc(*args, **kwargs):
    return run(compose + list(args), **kwargs)

def wait(check, label, seconds=90):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            if check():
                return
        except (OSError, ValueError):
            pass
        time.sleep(.5)
    raise RuntimeError("timed out: " + label)

api = "http://127.0.0.1:" + env["SAGAT_API_PORT"] + "/v1"
idp = "http://127.0.0.1:" + env["SAGAT_IDP_PORT"]
admin = None

def http(method, path, body=None, key=None, expected=200, identity=False):
    url = idp + path if identity else api + path
    headers = {"Content-Type":"application/json", "x-octopus-namespace":"payments"}
    if admin and not identity:
        headers["Authorization"] = "Bearer " + admin
    if method != "GET":
        headers["idempotency-key"] = key or str(uuid.uuid4())
    request = urllib.request.Request(url, method=method, headers=headers, data=None if body is None else json.dumps(body).encode())
    try:
        response = urllib.request.urlopen(request, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        payload = response.read(4 * 1024 * 1024)
        assert response.code == expected, f"unexpected HTTP status {response.code} on {method} {path}"
        return json.loads(payload) if payload else {}

try:
    run(["python3", "infra/openbao/prepare-development.py", "--directory", str(private), "--namespace-id", str(uuid.uuid4())])
    # Choose private networks not already present in this Docker daemon (including kind).
    import ipaddress
    existing = run(["docker", "network", "ls", "-q"]).stdout.decode().split()
    networks = json.loads(run(["docker", "network", "inspect", *existing]).stdout)
    occupied = [ipaddress.ip_network(c["Subnet"]) for network in networks for c in (network.get("IPAM", {}).get("Config") or []) if "Subnet" in c]
    selected = []
    for i in range(180, 250):
        subnet = ipaddress.ip_network(f"10.213.{i}.0/24")
        if not any(subnet.overlaps(other) for other in occupied):
            selected.append(str(subnet))
        if len(selected) == 2:
            break
    assert len(selected) == 2, "two isolated fixture subnets are required"
    env["SAGAT_DATABASE_ENGINE_SUBNET"], env["SAGAT_DATABASE_TARGET_SUBNET"] = selected
    config_path = private / "runner/config.json"
    config = json.loads(config_path.read_text())
    config["deployment"]["allowCidrs"] = [selected[1]]
    config["deployment"]["denyCidrs"] = [selected[0]]
    config["namespaces"][0]["allowCidrs"] = [selected[1]]
    config_path.write_text(json.dumps(config))
    dc("config", "--quiet")
    dc("up", "-d", "openbao", "target-postgres", "dev-idp", "migrate", "nats", timeout=SLOW)
    wait(lambda: http("GET", "/.well-known/jwks.json", identity=True), "local OIDC")
    run(["python3", "infra/openbao/bootstrap-development.py", "--directory", str(private), "--address", "https://127.0.0.1:" + env["SAGAT_BAO_PORT"], "--idp", idp])
    dc("--profile", "seed", "run", "--rm", "seed", timeout=SLOW)
    dc("up", "-d", "control-plane", "execution-engine", timeout=SLOW)
    wait(lambda: http("GET", "/ready"), "control-plane readiness")
    admin = http("POST", "/token", {"subject":"dev-admin", "tenant":"dev"}, identity=True)["access_token"]
    ns = next(n["id"] for n in http("GET", "/namespaces")["items"] if n["slug"] == "payments")
    config["namespaces"][0]["id"] = ns
    config_path.write_text(json.dumps(config))
    # Existing operator provisioning boundary. Membership and its audit commit together.
    sql = """BEGIN;
INSERT INTO memberships(tenant_id,subject,role,created_at) SELECT id,'database-runner','database-runner',clock_timestamp() FROM tenants WHERE slug='dev';
INSERT INTO audit_records(id,tenant_id,actor,action,resource_type,resource_id,reason,created_at) SELECT gen_random_uuid(),id,'system:fixture','membership.granted','membership','database-runner','database deployment proof',clock_timestamp() FROM tenants WHERE slug='dev';
COMMIT;"""
    dc("exec", "-T", "postgres", "psql", "-U", "sagat", "-d", "sagat", "-v", "ON_ERROR_STOP=1", data=sql.encode())
    http("PUT", f"/namespaces/{ns}/worker-bindings/database-runner", {"activities":["sagat.database.postgresql"]})
    http("PUT", f"/namespaces/{ns}/secrets/db-credential", {"value":(private / "credential.json").read_text(), "requestId":str(uuid.uuid4()), "expectedVersion":0, "requiredProvider":"openbao"})
    http("PUT", f"/namespaces/{ns}/connections/orders-db", {"requestId":str(uuid.uuid4()), "expectedRevision":0, "status":"ACTIVE", "config":{"driver":"postgresql", "host":"target-postgres", "port":5432, "database":"postgres", "secretName":"db-credential", "tls":{"mode":"verify-full", "caPem":(private / "tls/ca.crt").read_text()}}})
    dc("--profile", "database", "up", "-d", "database-runner", timeout=SLOW)
    def finished(run_id, status):
        def check():
            value = http("GET", "/runs/" + run_id)
            current = value["status"]
            if current in ("SUCCEEDED", "FAILED", "MANUAL_REVIEW", "CANCELLED", "TIMED_OUT"):
                assert current == status, "unexpected durable result " + current
                return True
            return False
        wait(check, "durable run " + status)
    probe = http("POST", f"/namespaces/{ns}/connections/orders-db/probe", {}, expected=201)
    finished(probe["runId"], "SUCCEEDED")
    definition = {"apiVersion":"sagat-octopus-orchestrator.io/v1", "kind":"Workflow", "metadata":{"name":"compose-query"}, "entrypoint":"query", "nodes":[{"id":"query", "type":"database", "connectionRef":{"name":"orders-db"}, "operation":"query", "statement":"SELECT id, status FROM demo_orders WHERE id=$1", "parameters":[{"literal":{"type":"integer","value":"1"}}]}]}
    body = {"mode":"step", "input":{}, "definition":definition}
    key = str(uuid.uuid4())
    admitted = http("POST", f"/namespaces/{ns}/database-executions", body, key=key, expected=201)
    replay = http("POST", f"/namespaces/{ns}/database-executions", body, key=key, expected=201)
    assert admitted["runId"] == replay["runId"]
    finished(admitted["runId"], "SUCCEEDED")
    result = http("GET", f"/namespaces/{ns}/runs/{admitted['runId']}/database-results/query")
    assert result["rows"] == [[{"type":"integer","value":"1"},{"type":"text","value":"ready"}]]
    print("PASS: built container runner, verified engine/target TLS, OpenBao secret, probe, retained typed result and admission replay", flush=True)
    def verify_restored():
        wait(lambda: http("GET", "/ready"), "restored control-plane")
        assert http("GET", f"/namespaces/{ns}/runs/{admitted['runId']}/database-results/query") == result
        assert http("POST", f"/namespaces/{ns}/database-executions", body, key=key, expected=201)["runId"] == admitted["runId"]
        fresh = http("POST", f"/namespaces/{ns}/database-executions", body, expected=201)
        finished(fresh["runId"], "SUCCEEDED")
    restore(dc, private, "https://127.0.0.1:" + env["SAGAT_BAO_PORT"], verify_restored)
    dc("stop", "openbao")
    failed = http("POST", f"/namespaces/{ns}/database-executions", body, expected=201)
    finished(failed["runId"], "FAILED")
    print("PASS: provider outage fails closed; no local credential fallback", flush=True)
finally:
    if (private / "compose.env").exists():
        dc("--profile", "database", "down", "-v", "--remove-orphans", check=False)
    work.cleanup()
