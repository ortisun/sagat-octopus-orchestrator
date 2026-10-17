#!/usr/bin/env python3
"""Operator bootstrap after init/unseal; never persists or prints the bootstrap token.

Use a short-lived operator token able to manage these mounts, policies, auth and audit.
Run with verified TLS against the leader. Namespace/fullname must match the Sagat release.
"""
import argparse
import json
import pathlib
import re
import ssl
import urllib.error
import urllib.parse
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--address", required=True)
    parser.add_argument("--ca", required=True)
    parser.add_argument("--token-file", required=True)
    parser.add_argument("--namespace", default="sagat")
    parser.add_argument("--fullname", default="sagat-octopus-orchestrator")
    parser.add_argument("--kubernetes-host", default="https://kubernetes.default.svc:443")
    args = parser.parse_args()
    address = urllib.parse.urlsplit(args.address)
    if address.scheme != "https" or not address.hostname or address.username or address.password or address.query or address.fragment or address.path not in ("", "/"):
        raise SystemExit("address must be an HTTPS origin")
    for value in (args.namespace, args.fullname):
        if not re.fullmatch(r"[a-z0-9]([-a-z0-9]*[a-z0-9])?", value):
            raise SystemExit("invalid Kubernetes resource name")
    token = pathlib.Path(args.token_file).read_text().strip()
    if not 1 <= len(token) <= 8192 or any(c.isspace() for c in token):
        raise SystemExit("invalid bootstrap token file")
    # No environment proxy and no redirects carrying authentication to a different endpoint.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            return None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect(),
        urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=args.ca)))

    def api(method, path, value=None):
        request = urllib.request.Request(args.address.rstrip("/") + "/v1/" + path,
            data=None if value is None else json.dumps(value).encode(), method=method,
            headers={"X-Vault-Token": token, "Content-Type": "application/json"})
        try:
            with opener.open(request, timeout=10) as response:
                data = response.read(1024 * 1024 + 1)
                if len(data) > 1024 * 1024:
                    raise SystemExit("bootstrap response exceeds limit")
                return json.loads(data) if data else {}
        except urllib.error.HTTPError as error:
            raise SystemExit(f"bootstrap {method} {path} failed: HTTP {error.code}") from None
        except (OSError, ValueError):
            raise SystemExit(f"bootstrap {method} {path} unavailable") from None

    mounts = api("GET", "sys/mounts")["data"]
    if "secret/" not in mounts:
        api("POST", "sys/mounts/secret", {"type": "kv", "options": {"version": "2"}})
    elif mounts["secret/"]["type"] != "kv" or mounts["secret/"].get("options", {}).get("version") != "2":
        raise SystemExit("existing secret mount must be KV v2")
    auth = api("GET", "sys/auth")["data"]
    if "kubernetes/" not in auth:
        api("POST", "sys/auth/kubernetes", {"type": "kubernetes"})
    elif auth["kubernetes/"]["type"] != "kubernetes":
        raise SystemExit("existing kubernetes mount has the wrong type")
    # OpenBao uses and renews its own local projected TokenReviewer identity. The chart grants
    # system:auth-delegator only to the OpenBao server SA; no application SA gets that right.
    api("POST", "auth/kubernetes/config", {"kubernetes_host": args.kubernetes_host})
    for policy, component in [("management", "control-plane"), ("runtime", "execution-engine"), ("cleanup", "secret-cleanup")]:
        name = f"sagat-{policy}"
        source = pathlib.Path(__file__).with_name("policies").joinpath(policy + ".hcl").read_text()
        api("PUT", "sys/policies/acl/" + name, {"policy": source})
        api("POST", "auth/kubernetes/role/" + name, {
            "bound_service_account_names": [args.fullname + "-" + component],
            "bound_service_account_namespaces": [args.namespace], "audience": "openbao",
            "token_policies": [name], "token_no_default_policy": True,
            "token_ttl": "30m", "token_max_ttl": "1h", "token_explicit_max_ttl": "1h",
        })
    audits = api("GET", "sys/audit")["data"]
    if audits.get("sagat/", {}).get("options", {}).get("file_path") != "/openbao/audit/audit.log":
        raise SystemExit("declarative sagat audit device must be enabled by the server profile")
    print("KV v2, Kubernetes workload roles and HMAC-redacted audit configured")


if __name__ == "__main__":
    main()
