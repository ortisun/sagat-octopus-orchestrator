#!/usr/bin/env python3
"""Real Kubernetes auth/CNI proof using the rendered production workload profiles.

The caller creates an isolated kind-sagat-db017* cluster with Calico and the pinned OpenBao
chart. Only this owned test context is accepted. Credentials stay in the private fixture dir.
The application containers are network probes here; SQL execution has its separate process suite.
"""
import argparse
import base64
import json
import os
import pathlib
import socket
import ssl
import subprocess
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[3]
BAO = "docker.io/openbao/openbao:2.7.1@sha256:6d2b93856e3fcf7b18ad855a0b51eaba474dc8b79cf554379ea32034797d2acf"
PYTHON = "python:3.13-alpine@sha256:2d9aefe2fef018a7eb2c13064c89c71929800fd2e5dccdbf52ea5da5bb8d929a"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--kubeconfig", required=True)
parser.add_argument("--directory", required=True)
args = parser.parse_args()
directory = pathlib.Path(args.directory)
directory.mkdir(mode=0o700, parents=True, exist_ok=True)
os.chmod(directory, 0o700)
k = ["kubectl", "--kubeconfig", args.kubeconfig]

def command(argv, data=None, check=True):
    result = subprocess.run(argv, input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=50)
    if check and result.returncode:
        # kubectl/bao errors can contain credentials; report the operation only.
        raise RuntimeError("fixture command failed: " + argv[0])
    return result

def kubectl(*argv, data=None, check=True):
    return command(k + list(argv), data, check)

def apply(value):
    kubectl("apply", "-f", "-", data=json.dumps(value).encode())

def wait(check, label, seconds=120):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        if check():
            return
        time.sleep(1)
    raise RuntimeError("timed out: " + label)

context = kubectl("config", "current-context").stdout.decode().strip()
assert context.startswith("kind-sagat-db017"), "only the isolated owned fixture cluster is allowed"
ca = directory / "tls.crt"
assert ca.is_file(), "caller must generate fixture TLS material before installing OpenBao"
context_tls = ssl.create_default_context(cafile=str(ca))
processes = []

def forward(pod):
    def running():
        result = kubectl("-n", "openbao", "get", "pod", pod, "-o", "json", check=False)
        if result.returncode:
            return False
        value = json.loads(result.stdout)
        return not value["metadata"].get("deletionTimestamp") and any("running" in c.get("state", {}) for c in value.get("status", {}).get("containerStatuses", []))
    wait(running, "OpenBao container " + pod)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    process = subprocess.Popen(k + ["-n", "openbao", "port-forward", "pod/" + pod, f"{port}:8200"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    processes.append(process)
    def listening():
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return True
        except OSError:
            return False
    wait(listening, "port-forward " + pod)
    return f"https://127.0.0.1:{port}"

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect(), urllib.request.HTTPSHandler(context=context_tls))

def api(address, method, path, body=None, token=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["X-Vault-Token"] = token
    request = urllib.request.Request(address + "/v1/" + path, method=method, headers=headers, data=None if body is None else json.dumps(body).encode())
    try:
        with opener.open(request, timeout=10) as response:
            payload = response.read(1024 * 1024)
            return response.status, json.loads(payload) if payload else {}
    except urllib.error.HTTPError as error:
        return error.code, {}
    except (OSError, TimeoutError):
        return 0, {}

def secret(name, values):
    apply({"apiVersion": "v1", "kind": "Secret", "metadata": {"name": name, "namespace": "sagat"}, "data": {key: base64.b64encode(value).decode() for key, value in values.items()}})

def render(template):
    output = command(["helm", "template", "sagat", str(ROOT / "infra/helm"), "--namespace", "sagat", "-f", str(ROOT / "infra/helm/examples/ha.yaml"), "-f", str(ROOT / "infra/helm/examples/database.yaml"), "--set", "databaseRunner.image=" + BAO, "--set", "engine.autoscaling.keda.enabled=false", "--set-json", "workerPools=[]", "--show-only", "templates/" + template]).stdout
    remaining = kubectl("create", "--dry-run=client", "--validate=false", "-f", "-", "-o", "json", data=output).stdout.decode().strip()
    documents = []
    while remaining:
        decoded, offset = json.JSONDecoder().raw_decode(remaining)
        documents.extend(decoded.get("items", [decoded]))
        remaining = remaining[offset:].strip()
    return documents

def ready(name):
    result = kubectl("-n", "sagat", "get", "pod", name, "-o", "json", check=False)
    if result.returncode:
        return False
    return any(c["type"] == "Ready" and c["status"] == "True" for c in json.loads(result.stdout).get("status", {}).get("conditions", []))

def cli(name, script, expected=True):
    result = kubectl("-n", "sagat", "exec", name, "-c", "probe", "--", "sh", "-ec", script, check=False)
    assert (result.returncode == 0) == expected, "unexpected authorization/network result: " + name
    return result.stdout

try:
    addresses = [forward("openbao-" + str(i)) for i in range(3)]
    state = directory / "init.json"
    if state.exists():
        init = json.loads(state.read_text())
    else:
        status, init = api(addresses[0], "PUT", "sys/init", {"secret_shares": 1, "secret_threshold": 1})
        assert status == 200, "fixture initialization"
        state.write_text(json.dumps(init)); os.chmod(state, 0o600)
    for address in addresses:
        def unsealed():
            status, value = api(address, "PUT", "sys/unseal", {"key": init["keys_base64"][0]})
            return status == 200 and value.get("sealed") is False
        wait(unsealed, "raft join/unseal")
    for address in addresses:
        wait(lambda: api(address, "GET", "sys/health?standbyok=true")[0] == 200, "post-unseal setup")
    root_file = directory / "bootstrap-token"
    root_file.write_text(init["root_token"]); os.chmod(root_file, 0o600)
    command(["python3", str(ROOT / "infra/openbao/bootstrap-kubernetes.py"), "--address", addresses[0], "--ca", str(ca), "--token-file", str(root_file)])
    print("PASS: verified TLS, three Raft members, bootstrap and audit", flush=True)
    # Short fixture TTLs prove Agent renewal/re-authentication without waiting an hour.
    for role in ["management", "runtime"]:
        status, response = api(addresses[0], "GET", "auth/kubernetes/role/sagat-" + role, token=init["root_token"])
        assert status == 200
        config = response["data"]
        config.update(token_ttl="8s", token_max_ttl="16s", token_explicit_max_ttl="16s")
        assert api(addresses[0], "POST", "auth/kubernetes/role/sagat-" + role, config, init["root_token"])[0] in (200, 204)
    apply({"apiVersion":"v1", "kind":"Namespace", "metadata":{"name":"sagat"}})
    secret("openbao-ca", {"ca.crt": ca.read_bytes()})
    secret("openbao-intent", {"intent-key": os.urandom(32)})
    for resource in render("openbao-agent.yaml"):
        resource["metadata"]["namespace"] = "sagat"; apply(resource)
    for template, name in [("control-plane.yaml", "management"), ("execution-engine.yaml", "runtime"), ("secret-cleanup.yaml", "cleanup")]:
        docs = render(template)
        workload = next(d for d in docs if d["kind"] in ("Deployment", "CronJob"))
        template_spec = workload["spec"]["jobTemplate"]["spec"]["template"] if name == "cleanup" else workload["spec"]["template"]
        pod = template_spec["spec"]
        pod["restartPolicy"] = "Always"
        # Use only the production agent and provider mounts. No platform DB/password is provisioned.
        app = pod["containers"][0]
        app = {key: value for key, value in app.items() if key in ("securityContext", "resources", "volumeMounts")}
        app.update(name="probe", image=BAO, command=["sh", "-ec", "sleep 3600"], env=[{"name":"BAO_ADDR", "value":"https://openbao.openbao.svc:8200"}, {"name":"BAO_CACERT", "value":"/etc/openbao/ca/ca.crt"}])
        app["volumeMounts"] = [m for m in app["volumeMounts"] if m["name"].startswith("openbao-")]
        pod["containers"] = [app]
        pod["volumes"] = [v for v in pod["volumes"] if v["name"].startswith("openbao-")]
        apply({"apiVersion":"v1", "kind":"Pod", "metadata":{"name":name, "namespace":"sagat", "labels":template_spec["metadata"]["labels"]}, "spec":pod})
    for name in ("management", "runtime", "cleanup"):
        wait(lambda: ready(name), "agent login " + name)
    prefix = 'export BAO_TOKEN="$(cat /run/openbao/token)"; '
    path = "secret/sagat/credentials/tenant/ns/fixture"
    cli("management", prefix + f"bao kv put -cas=0 {path} value=credential-canary >/dev/null")
    assert b"credential-canary" in cli("runtime", prefix + f"bao kv get -format=json {path}")
    cli("runtime", prefix + f"bao kv put -cas=0 {path}-denied value=denied", False)
    cli("cleanup", prefix + f"bao kv get {path}", False)
    cli("management", prefix + f"bao kv destroy -versions=1 {path}", False)
    cli("cleanup", prefix + f"bao kv destroy -versions=1 {path} >/dev/null")
    cli("cleanup", prefix + f"bao kv metadata delete {path}", False)
    cli("management", prefix + "bao token lookup -format=json")
    wait(lambda: b'"ttl":' in cli("management", prefix + "bao token lookup -format=json"), "finite token")
    first = cli("management", "sha256sum /run/openbao/token").split()[0]
    wait(lambda: cli("management", "sha256sum /run/openbao/token").split()[0] != first, "agent re-authentication", 40)
    cli("management", prefix + "bao token lookup >/dev/null")
    print("PASS: projected JWT login, finite tokens, renewal/re-authentication, isolated ACLs", flush=True)
    # Wrong audience and wrong SA cannot exchange their Kubernetes identity for a provider token.
    for sa, audience in [("sagat-octopus-orchestrator-control-plane", "wrong-audience"), ("default", "openbao")]:
        jwt = kubectl("-n", "sagat", "create", "token", sa, "--audience", audience, "--duration=10m").stdout.decode().strip()
        assert api(addresses[0], "POST", "auth/kubernetes/login", {"role":"sagat-management", "jwt":jwt})[0] == 403
    # Apply actual generated provider and runner network policies, then probe real sockets.
    api_ip = json.loads(kubectl("get", "service", "kubernetes", "-o", "json").stdout)["spec"]["clusterIP"]
    endpoints = json.loads(kubectl("get", "endpoints", "kubernetes", "-o", "json").stdout)
    cidrs = [api_ip + "/32"] + [a["ip"] + "/32" for subset in endpoints["subsets"] for a in subset["addresses"]]
    policy = json.loads(command(["python3", str(ROOT / "infra/openbao/render-network.py")] + [arg for cidr in cidrs for arg in ("--kubernetes-cidr", cidr)]).stdout)
    apply(policy)
    probes = [("target", 5432), ("platform-state", 5432), ("engine-probe", 9000)]
    for name, port in probes:
        labels = {"test-service":name}
        if name == "engine-probe":
            labels = {"app.kubernetes.io/name":"sagat-octopus-orchestrator", "app.kubernetes.io/instance":"sagat", "app.kubernetes.io/component":"execution-engine"}
        apply({"apiVersion":"v1", "kind":"Pod", "metadata":{"name":name, "namespace":"sagat", "labels":labels}, "spec":{"automountServiceAccountToken":False, "containers":[{"name":"listener", "image":PYTHON, "command":["python3", "-m", "http.server", str(port)], "securityContext":{"runAsUser":10001, "runAsNonRoot":True, "allowPrivilegeEscalation":False, "capabilities":{"drop":["ALL"]}}, "resources":{"requests":{"cpu":"10m","memory":"24Mi"},"limits":{"cpu":"100m","memory":"64Mi"}}}]}})
        wait(lambda: ready(name), "network listener " + name)
    docs = render("database-runner.yaml")
    network = next(d for d in docs if d["kind"] == "NetworkPolicy")
    # Lab target is a selected pod, replacing only the example external CIDR destination.
    network["spec"]["egress"][-1]["to"] = [{"podSelector":{"matchLabels":{"test-service":"target"}}}]
    network["metadata"]["namespace"] = "sagat"; apply(network)
    labels = next(d for d in docs if d["kind"] == "StatefulSet")["spec"]["template"]["metadata"]["labels"]
    apply({"apiVersion":"v1", "kind":"Pod", "metadata":{"name":"runner-probe", "namespace":"sagat", "labels":labels}, "spec":{"automountServiceAccountToken":False,"containers":[{"name":"probe","image":BAO,"command":["sh","-ec","sleep 3600"],"resources":{"limits":{"cpu":"100m","memory":"64Mi"}},"securityContext":{"runAsUser":10001,"runAsNonRoot":True,"allowPrivilegeEscalation":False,"capabilities":{"drop":["ALL"]}}}]}})
    wait(lambda: ready("runner-probe"), "network probe")
    time.sleep(3)
    for name, port in probes:
        ip = json.loads(kubectl("-n", "sagat", "get", "pod", name, "-o", "json").stdout)["status"]["podIP"]
        cli("runner-probe", f"nc -z -w 2 {ip} {port}", name != "platform-state")
    cli("runner-probe", "nc -z -w 2 openbao.openbao.svc 8200", False)
    cli("runner-probe", f"nc -z -w 2 {api_ip} 443", False)
    cli("runner-probe", "nc -z -w 2 169.254.169.254 80", False)
    # A renewed agent still reaches TokenReview after provider egress becomes default-deny.
    first = cli("management", "sha256sum /run/openbao/token").split()[0]
    wait(lambda: cli("management", "sha256sum /run/openbao/token").split()[0] != first, "reauthentication under CNI policy", 40)
    cli("management", prefix + "bao token lookup >/dev/null")
    print("PASS: Calico target/engine allowed; platform/OpenBao/API/metadata denied; TokenReview works", flush=True)
finally:
    for process in processes:
        process.terminate()
    for process in processes:
        process.wait(timeout=5)
