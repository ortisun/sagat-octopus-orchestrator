# OpenBao deployment and bootstrap

## Overview

Operator scripts, local private fixtures, production chart values and scoped capability policies for managed database credentials.

## Interface

- `prepare-development.py --directory --namespace-id [--namespace-slug]` creates the local TLS/database/runner fixture. `bootstrap-development.py --directory [--address --idp --tenant]` initializes/unseals it and refreshes scoped tokens. [prepare-development.py:parser] [bootstrap-development.py:parser]
- `bootstrap-kubernetes.py` configures KV v2, Kubernetes authentication, policies and workload roles through `main`; it accepts address, CA, token file, namespace, fullname and Kubernetes host flags. [bootstrap-kubernetes.py:main]
- `fetch-chart.py OUTPUT.tgz` fetches chart `VERSION` and validates `SHA256`; `render-network.py` prints NetworkPolicy JSON from explicit Kubernetes CIDRs, port and namespace. [fetch-chart.py:VERSION] [fetch-chart.py:SHA256] [render-network.py:parser]
- `production-values.yaml` supplies the reviewed chart settings; the [policy contracts](./policies/README.md) list management, runtime and cleanup grants.

## Invariants

- Local preparation refuses an existing directory, creates a private root, and rejects overlapping engine/target networks. Development bootstrap accepts only numeric loopback endpoints and requires the private fixture directory. [prepare-development.py:root.mkdir] [prepare-development.py:overlaps] [bootstrap-development.py:is_loopback]
- Bootstrap HTTP clients disable environment proxies and redirects and use verified TLS. Kubernetes bootstrap bounds response size, validates resource names/token shape, and refuses incompatible existing mounts. [bootstrap-development.py:NoRedirect] [bootstrap-kubernetes.py:NoRedirect] [bootstrap-kubernetes.py:api]
- Kubernetes workload roles bind named service accounts and namespace, use audience `openbao`, no default policy, a 30-minute TTL and one-hour maximum TTL. Network rendering refuses global CIDRs and invalid ports. [bootstrap-kubernetes.py:token_ttl] [bootstrap-kubernetes.py:bound_service_account_names] [render-network.py:prefixlen]

## Side Effects

- Preparation invokes OpenSSL and writes keys, certificates, generated credentials, fixture SQL and service configuration into the selected fixture directory. [prepare-development.py:openssl] [prepare-development.py:write]
- Bootstrap scripts issue authenticated OpenBao calls that create/configure mounts, policies and roles. Development bootstrap additionally initializes/unseals, obtains an IdP token and writes local state/token files. [bootstrap-development.py:call] [bootstrap-development.py:save] [bootstrap-kubernetes.py:api]
- Chart fetching downloads and exclusively creates the requested archive after checksum verification. Network rendering writes JSON to stdout. [fetch-chart.py:urlopen] [fetch-chart.py:open] [render-network.py:print]

## Verification

Run from the repository root:

```sh
cargo test -p sagat-tests --test integration_database_connections --locked
```

Commands are verification entry points, not results from this documentation pass. Use the repository resource profile in [AGENTS.md](../../AGENTS.md) for Cargo builds; integration/drill commands need their declared services and tools.
