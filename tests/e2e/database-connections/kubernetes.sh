#!/usr/bin/env bash
# Reproducible, isolated real TLS/Raft/Kubernetes-auth/CNI test. No existing context is modified.
set -euo pipefail
umask 077
repo=$(cd "$(dirname "$0")/../../.." && pwd)
fixture=$(mktemp -d "${TMPDIR:-/tmp}/sagat-db017.XXXXXX")
cluster="sagat-db017-$(date +%s)"
kubeconfig="$fixture/kubeconfig"
cleanup() {
  kind delete cluster --name "$cluster" >/dev/null 2>&1 || true
  rm -rf "$fixture"
}
trap cleanup EXIT
cat > "$fixture/kind.yaml" <<'YAML'
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
networking:
  disableDefaultCNI: true
  podSubnet: 192.168.0.0/16
nodes:
  - role: control-plane
  - role: worker
  - role: worker
YAML
python3 "$repo/infra/openbao/fetch-chart.py" "$fixture/openbao.tgz"
curl --fail --silent --show-error --location --max-time 30 \
  https://raw.githubusercontent.com/projectcalico/calico/v3.31.4/manifests/calico.yaml -o "$fixture/calico.yaml"
printf '%s  %s\n' 6e89ee080d6dae58c620c47e3fcb7a9bfc40748422c868c924b7482dd5145ce2 "$fixture/calico.yaml" | shasum -a 256 --check
kind create cluster --name "$cluster" --kubeconfig "$kubeconfig" --config "$fixture/kind.yaml" \
  --image kindest/node:v1.35.8@sha256:07b2536e30b803ed61d1677a79df6115f798ce64c80f9e22f6ed45afd09323c0
k=(kubectl --kubeconfig "$kubeconfig")
"${k[@]}" apply -f "$fixture/calico.yaml" >/dev/null
# The lab has three nodes including control-plane; production uses three worker nodes.
"${k[@]}" taint node "$cluster-control-plane" node-role.kubernetes.io/control-plane- >/dev/null
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$fixture/tls.key" -out "$fixture/tls.crt" \
  -days 2 -subj /CN=openbao.openbao.svc \
  -addext 'subjectAltName=DNS:openbao.openbao.svc,DNS:*.openbao-internal.openbao.svc,DNS:localhost,IP:127.0.0.1' >/dev/null 2>&1
"${k[@]}" create namespace openbao >/dev/null
"${k[@]}" -n openbao create secret generic openbao-tls --from-file="tls.crt=$fixture/tls.crt" \
  --from-file="tls.key=$fixture/tls.key" --from-file="ca.crt=$fixture/tls.crt" >/dev/null
helm install openbao "$fixture/openbao.tgz" --kubeconfig "$kubeconfig" --namespace openbao \
  -f "$repo/infra/openbao/production-values.yaml" >/dev/null
python3 "$repo/tests/e2e/database-connections/kubernetes.py" --kubeconfig "$kubeconfig" --directory "$fixture"
