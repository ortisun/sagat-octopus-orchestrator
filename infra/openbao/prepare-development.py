#!/usr/bin/env python3
"""Generate private local-only fixture files. Refuses to replace an existing environment."""
import argparse
import ipaddress
import json
import os
import pathlib
import secrets
import subprocess
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--directory", required=True)
parser.add_argument("--namespace-id", required=True, type=uuid.UUID)
parser.add_argument("--namespace-slug", default="payments")
args = parser.parse_args()
root = pathlib.Path(args.directory).resolve()
root.mkdir(mode=0o700, parents=True, exist_ok=False)
for name in ("tls", "bao", "management", "runtime", "cleanup", "intent", "target-init", "runner", "identity"):
    (root / name).mkdir(mode=0o755)

def write(path, value, mode=0o644):
    path.write_bytes(value if isinstance(value, bytes) else value.encode())
    path.chmod(mode)

def openssl(*argv):
    subprocess.run(["openssl", *map(str, argv)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

tls = root / "tls"
openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", tls / "ca.key", "-out", tls / "ca.crt", "-days", "7", "-subj", "/CN=Sagat local database fixture CA", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-addext", "subjectKeyIdentifier=hash")
(tls / "ca.key").chmod(0o600)
for name, hosts in [("bao", "DNS:openbao,DNS:localhost,IP:127.0.0.1"), ("engine", "DNS:execution-engine,DNS:localhost,IP:127.0.0.1"), ("target", "DNS:target-postgres")]:
    extension = tls / (name + ".ext")
    write(extension, "subjectAltName=" + hosts + "\nextendedKeyUsage=serverAuth\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\n")
    openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", tls / (name + ".key"), "-out", tls / (name + ".csr"), "-subj", "/CN=" + name)
    openssl("x509", "-req", "-in", tls / (name + ".csr"), "-CA", tls / "ca.crt", "-CAkey", tls / "ca.key", "-CAcreateserial", "-out", tls / (name + ".crt"), "-days", "7", "-extfile", extension)
    # The enclosing host directory is 0700. Bind only the specific leaf files into each container.
    (tls / (name + ".key")).chmod(0o644)
write(root / "intent/intent-key", secrets.token_bytes(32))
password = secrets.token_hex(24)
write(root / "target.env", "POSTGRES_PASSWORD=" + secrets.token_hex(24) + "\n", 0o600)
write(root / "target-init/01-fixture.sql", f"CREATE ROLE sagat_target LOGIN PASSWORD '{password}';\nCREATE TABLE demo_orders(id bigint PRIMARY KEY, status text NOT NULL);\nINSERT INTO demo_orders VALUES (1, 'ready');\nGRANT SELECT, INSERT, UPDATE, DELETE ON demo_orders TO sagat_target;\n")
write(root / "credential.json", json.dumps({"username":"sagat_target", "password":password}), 0o600)
write(root / "bao/server.hcl", '''ui = false
disable_mlock = true
api_addr = "https://openbao:8200"
cluster_addr = "https://openbao:8201"
listener "tcp" {
  address = "0.0.0.0:8200"
  cluster_address = "0.0.0.0:8201"
  tls_cert_file = "/openbao/tls/bao.crt"
  tls_key_file = "/openbao/tls/bao.key"
  tls_min_version = "tls12"
}
storage "raft" { path = "/openbao/data" }
audit "file" "sagat" {
  options {
    file_path = "stdout"
    log_raw = "false"
  }
}
''')
engine = str(ipaddress.ip_network(os.environ.get("SAGAT_DATABASE_ENGINE_SUBNET", "10.213.16.0/24")))
target = str(ipaddress.ip_network(os.environ.get("SAGAT_DATABASE_TARGET_SUBNET", "10.213.17.0/24")))
assert not ipaddress.ip_network(engine).overlaps(ipaddress.ip_network(target)), "fixture networks must not overlap"
limits = {"maxStatementBytes":16384,"maxParameters":64,"maxParameterBytes":65536,"maxRows":1000,"maxResultBytes":1048576,"timeoutMs":10000,"connectTimeoutMs":5000,"poolSize":4,"idleTimeoutMs":30000}
config = {"grpcUrl":"https://execution-engine:9000", "tokenFile":"/run/sagat-identity/token", "caFile":"/etc/sagat/ca.crt", "name":"database-compose", "maxConcurrent":4,"pollIntervalMs":250,"rpcTimeoutMs":5000,"deployment":{"allowCidrs":[target],"allowPorts":[5432],"denyCidrs":[engine],"limits":limits},"namespaces":[{"id":str(args.namespace_id),"slug":args.namespace_slug,"allowCidrs":[target],"allowPorts":[5432],"limits":limits}]}
write(root / "runner/config.json", json.dumps(config))
write(root / "compose.env", f"SAGAT_DATABASE_DEV_DIR={root}\nSAGAT_DATABASE_ENGINE_SUBNET={engine}\nSAGAT_DATABASE_TARGET_SUBNET={target}\nSAGAT_DATABASE_POSTGRES_IMAGE=postgres:16-alpine@sha256:cf78e76683b9ca8c5733cbbdce6c9262b45b6767934dd0a95e671f9a0fc20685\n", 0o600)
print("Prepared local TLS, restricted PostgreSQL role and runner policy; bootstrap OpenBao next.")
