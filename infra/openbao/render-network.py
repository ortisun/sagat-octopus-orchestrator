#!/usr/bin/env python3
"""Render default-deny OpenBao network policy with explicit TokenReview API endpoints."""
import argparse
import ipaddress
import json

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--kubernetes-cidr", action="append", required=True,
    help="API ClusterIP and real endpoint CIDRs, repeat for pre/post-DNAT enforcement")
parser.add_argument("--kubernetes-port", type=int, default=6443)
parser.add_argument("--sagat-namespace", default="sagat")
args = parser.parse_args()
if not 1 <= args.kubernetes_port <= 65535:
    raise SystemExit("invalid API port")
cidrs = [ipaddress.ip_network(value, strict=True) for value in args.kubernetes_cidr]
if any(network.prefixlen == 0 for network in cidrs):
    raise SystemExit("global API destinations are refused")
def ns(name):
    return {"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": name}}}
server = {"podSelector": {"matchLabels": {"app.kubernetes.io/name": "openbao", "app.kubernetes.io/instance": "openbao", "component": "server"}}}
clients = {**ns(args.sagat_namespace), "podSelector": {"matchLabels": {"app.kubernetes.io/instance": "sagat"}, "matchExpressions": [{"key": "app.kubernetes.io/component", "operator": "In", "values": ["control-plane", "execution-engine", "secret-cleanup"]}]}}
dns = {**ns("kube-system"), "podSelector": {"matchLabels": {"k8s-app": "kube-dns"}}}
policy = {"apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy", "metadata": {"name": "openbao", "namespace": "openbao"}, "spec": {
    "podSelector": server["podSelector"], "policyTypes": ["Ingress", "Egress"],
    "ingress": [{"from": [server], "ports": [{"port": 8200}, {"port": 8201}]}, {"from": [clients], "ports": [{"port": 8200}]}],
    "egress": [{"to": [server], "ports": [{"port": 8200}, {"port": 8201}]},
        {"to": [dns], "ports": [{"port": 53, "protocol": "UDP"}, {"port": 53, "protocol": "TCP"}]},
        {"to": [{"ipBlock": {"cidr": str(network)}} for network in cidrs], "ports": [{"port": 443}, {"port": args.kubernetes_port}]}]}}
print(json.dumps(policy))
