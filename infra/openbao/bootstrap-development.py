#!/usr/bin/env python3
"""Initialize/unseal the isolated TLS development server and refresh one-hour scoped tokens.

The local root/unseal fixture stays in the 0700 directory. Production uses the operator
ceremony and Kubernetes authentication described in the runbook, never this script.
"""
import argparse
import ipaddress
import json
import os
import pathlib
import ssl
import time
import urllib.error
import urllib.parse
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--directory", required=True)
parser.add_argument("--address", default="https://127.0.0.1:18200")
parser.add_argument("--idp", default="http://127.0.0.1:18090")
parser.add_argument("--tenant", default="dev")
args = parser.parse_args()
root = pathlib.Path(args.directory).resolve()
assert root.is_dir() and root.stat().st_mode & 0o077 == 0, "fixture directory must be private (0700)"
for url, scheme in [(args.address, "https"), (args.idp, "http")]:
    parsed = urllib.parse.urlsplit(url)
    assert parsed.scheme == scheme and ipaddress.ip_address(parsed.hostname).is_loopback and not parsed.username and not parsed.password and not parsed.query and not parsed.fragment and parsed.path in ("", "/"), "local numeric-loopback fixture endpoints only"
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect(), urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(root / "tls/ca.crt"))))

def call(method, path, body=None, token=None, idp=False):
    headers = {"Content-Type":"application/json"}
    if token:
        headers["X-Vault-Token"] = token
    url = args.idp + path if idp else args.address + "/v1/" + path
    request = urllib.request.Request(url, method=method, headers=headers, data=None if body is None else json.dumps(body).encode())
    try:
        with opener.open(request, timeout=45) as response:
            data = response.read(1024 * 1024)
            return json.loads(data) if data else {}
    except urllib.error.HTTPError as error:
        raise SystemExit(f"development bootstrap {method} {path} failed: HTTP {error.code}") from None
    except (OSError, ValueError):
        raise SystemExit(f"development bootstrap {method} {path} unavailable; check endpoint health and TLS") from None

def save(path, value, mode=0o600):
    temporary = path.with_suffix(".new")
    temporary.write_text(value); temporary.chmod(mode); temporary.replace(path)

state = root / "development-unseal.json"
if state.exists():
    init = json.loads(state.read_text())
else:
    init = call("PUT", "sys/init", {"secret_shares":1,"secret_threshold":1})
    save(state, json.dumps(init))
call("PUT", "sys/unseal", {"key":init["keys_base64"][0]})
time.sleep(2)
token = init["root_token"]
mounts = call("GET", "sys/mounts", token=token)["data"]
if "secret/" not in mounts:
    call("POST", "sys/mounts/secret", {"type":"kv","options":{"version":"2"}}, token)
else:
    assert mounts["secret/"]["type"] == "kv" and mounts["secret/"]["options"]["version"] == "2"
for role in ("management", "runtime", "cleanup"):
    policy = pathlib.Path(__file__).with_name("policies").joinpath(role + ".hcl").read_text()
    call("PUT", "sys/policies/acl/sagat-" + role, {"policy":policy}, token)
    issued = call("POST", "auth/token/create", {"policies":["sagat-" + role],"no_default_policy":True,"renewable":False,"ttl":"1h","explicit_max_ttl":"1h"}, token)
    save(root / role / "token", issued["auth"]["client_token"], 0o644)
assert "sagat/" in call("GET", "sys/audit", token=token)["data"], "declarative audit device required"
jwt = call("POST", "/token", {"subject":"database-runner","tenant":args.tenant,"ttlSeconds":3600}, idp=True)
save(root / "identity/token", jwt["access_token"], 0o644)
print("Development OpenBao initialized/unsealed; scoped tokens refreshed for one hour.")
print("Grant database-runner its machine role and namespace binding before starting its profile.")
