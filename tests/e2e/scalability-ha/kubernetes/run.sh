#!/usr/bin/env bash
# Installs the actual product chart and proves its KEDA worker scaling from persisted Sagat work.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)
fixture_dir="$repo_root/tests/e2e/scalability-ha/kubernetes"
docker_context=${G247_DOCKER_CONTEXT:-colima-sagat-remaining}
cluster="sagat-g247-$(date +%s)-$$"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/sagat-g247.XXXXXX")
export KUBECONFIG="$work_dir/kubeconfig"
export HELM_REPOSITORY_CONFIG="$work_dir/helm-repositories.yaml"
export HELM_REPOSITORY_CACHE="$work_dir/helm-cache"
docker_host=$(docker context inspect --format '{{.Endpoints.docker.Host}}' "$docker_context")
export DOCKER_HOST="$docker_host"
unset DOCKER_CONTEXT

cluster_created=false
port_forward_pid=""
cleanup() {
  status=$?
  if [[ -n "$port_forward_pid" ]]; then
    kill "$port_forward_pid" >/dev/null 2>&1 || true
    wait "$port_forward_pid" >/dev/null 2>&1 || true
  fi
  if [[ "$cluster_created" == true && "${G247_KEEP_CLUSTER:-0}" != 1 ]]; then
    kind delete cluster --name "$cluster" >/dev/null 2>&1 || true
  fi
  if [[ "${G247_KEEP_CLUSTER:-0}" != 1 ]]; then
    rm -rf -- "$work_dir"
  else
    printf 'kept cluster %s and kubeconfig %s\n' "$cluster" "$KUBECONFIG" >&2
  fi
  exit "$status"
}
trap cleanup EXIT HUP INT TERM

for command_name in curl docker helm jq kind kubectl mktemp; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command_name" >&2
    exit 2
  }
done

build_image() {
  target=$1
  tag=$2
  docker build --resource cpu-quota=200000 --resource memory=3g --progress=plain \
    -f "$repo_root/infra/docker/Dockerfile" --target "$target" -t "$tag" "$repo_root"
}

if [[ "${G247_SKIP_BUILD:-0}" != 1 ]]; then
  build_image execution-engine sagat-octopus-orchestrator/execution-engine:g247-local
  build_image dev-idp sagat-octopus-orchestrator/dev-idp:g247-local
  build_image control-plane sagat-octopus-orchestrator/control-plane:g247-local
  docker build --resource cpu-quota=200000 --resource memory=2g --progress=plain \
    -f "$fixture_dir/Dockerfile.worker" -t sagat/g247-worker:local "$repo_root"
fi

for image in \
  sagat-octopus-orchestrator/execution-engine:g247-local \
  sagat-octopus-orchestrator/dev-idp:g247-local \
  sagat-octopus-orchestrator/control-plane:g247-local \
  sagat/g247-worker:local; do
  docker image inspect "$image" >/dev/null
done

kind create cluster --name "$cluster" --config "$fixture_dir/kind.yaml" --kubeconfig "$KUBECONFIG"
cluster_created=true

helm install cilium oci://quay.io/cilium/charts/cilium \
  --version 1.20.2 --namespace kube-system \
  --set ipam.mode=kubernetes --set operator.replicas=1 \
  --set image.pullPolicy=IfNotPresent --wait --timeout 7m
kubectl wait --for=condition=Ready nodes --all --timeout=180s

helm install keda keda --repo https://kedacore.github.io/charts \
  --version 2.18.1 --namespace keda --create-namespace --wait --timeout 7m
kubectl wait --namespace keda --for=condition=Available deployment/keda-operator --timeout=180s

for image in \
  sagat-octopus-orchestrator/execution-engine:g247-local \
  sagat-octopus-orchestrator/dev-idp:g247-local \
  sagat-octopus-orchestrator/control-plane:g247-local \
  sagat/g247-worker:local; do
  kind load docker-image --name "$cluster" "$image"
done

kubectl create namespace sagat-g247
kubectl label namespace sagat-g247 \
  pod-security.kubernetes.io/enforce=restricted \
  pod-security.kubernetes.io/enforce-version=latest --overwrite
kubectl apply --namespace sagat-g247 -f "$fixture_dir/dependencies.yaml"
kubectl wait --namespace sagat-g247 --for=condition=Available \
  deployment/postgres deployment/dev-idp deployment/prometheus --timeout=240s

local_port=$((19100 + ($$ % 500)))
kubectl --namespace sagat-g247 port-forward service/dev-idp "$local_port:9100" \
  >"$work_dir/idp-port-forward.log" 2>&1 &
port_forward_pid=$!
for _ in $(seq 1 30); do
  curl -fsS -H 'Host: dev-idp:9100' "http://127.0.0.1:$local_port/livez" >/dev/null 2>&1 && break
  sleep 1
done
token=$(curl -fsS -H 'Host: dev-idp:9100' -H 'content-type: application/json' \
  --data '{"subject":"dev-admin","tenant":"dev","ttlSeconds":3600}' \
  "http://127.0.0.1:$local_port/token" | jq -er .access_token)
printf '%s' "$token" \
  | kubectl --namespace sagat-g247 create secret generic g247-token \
      --from-file=token=/dev/stdin --dry-run=client -o yaml \
  | kubectl apply -f - >/dev/null
unset token
kill "$port_forward_pid" >/dev/null 2>&1 || true
wait "$port_forward_pid" >/dev/null 2>&1 || true
port_forward_pid=""

kubectl apply --namespace sagat-g247 -f "$fixture_dir/setup-jobs.yaml"
kubectl wait --namespace sagat-g247 --for=condition=Complete job/sagat-migrate --timeout=240s
kubectl apply --namespace sagat-g247 -f "$fixture_dir/seed-job.yaml"
kubectl wait --namespace sagat-g247 --for=condition=Complete job/sagat-seed --timeout=240s

helm upgrade --install sagat "$repo_root/infra/helm" --namespace sagat-g247 \
  --values "$fixture_dir/values.yaml" --timeout 7m
kubectl wait --namespace sagat-g247 --for=condition=Available \
  deployment/sagat-control-plane deployment/sagat-engine --timeout=240s

cat <<'YAML' | kubectl apply --namespace sagat-g247 -f -
apiVersion: batch/v1
kind: Job
metadata: {name: g247-network-denied}
spec:
  backoffLimit: 0
  template:
    metadata: {labels: {app.kubernetes.io/component: intruder}}
    spec:
      automountServiceAccountToken: false
      restartPolicy: Never
      securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
      containers:
        - name: probe
          image: sagat/g247-worker:local
          imagePullPolicy: Never
          command: [python]
          args:
            - -c
            - |
              import socket
              socket.setdefaulttimeout(3)
              try:
                  socket.create_connection(("sagat-control-plane", 8080))
              except OSError:
                  print("G247_UNDECLARED_API_DENIED=1", flush=True)
                  raise SystemExit(0)
              raise SystemExit("undeclared pod reached the control-plane API")
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            runAsNonRoot: true
            runAsUser: 10001
            capabilities: {drop: [ALL]}
YAML
kubectl wait --namespace sagat-g247 --for=condition=Complete job/g247-network-denied --timeout=60s

cat <<'YAML' | kubectl apply --namespace sagat-g247 -f -
apiVersion: batch/v1
kind: Job
metadata: {name: g247-enqueue}
spec:
  backoffLimit: 1
  template:
    metadata: {labels: {app.kubernetes.io/component: validator}}
    spec:
      automountServiceAccountToken: false
      restartPolicy: Never
      securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
      containers:
        - name: enqueue
          image: sagat/g247-worker:local
          imagePullPolicy: Never
          command: [python]
          args: [/opt/sagat/scenario.py, enqueue]
          env:
            - {name: SAGAT_SERVER, value: "http://sagat-control-plane:8080/v1"}
            - {name: SAGAT_NAMESPACE, value: payments}
            - {name: G247_RUNS, value: "60"}
            - name: SAGAT_TOKEN
              valueFrom: {secretKeyRef: {name: g247-token, key: token}}
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            runAsNonRoot: true
            runAsUser: 10001
            capabilities: {drop: [ALL]}
YAML
kubectl wait --namespace sagat-g247 --for=condition=Complete job/g247-enqueue --timeout=180s
kubectl wait --namespace sagat-g247 --for=condition=Available \
  deployment/sagat-worker-g247 --timeout=240s

prometheus_port=$((19600 + ($$ % 300)))
kubectl --namespace sagat-g247 port-forward service/prometheus "$prometheus_port:9090" \
  >"$work_dir/prometheus-port-forward.log" 2>&1 &
port_forward_pid=$!
for _ in $(seq 1 30); do
  curl -fsS "http://127.0.0.1:$prometheus_port/-/ready" >/dev/null 2>&1 && break
  sleep 1
done

peak_ready=0
peak_replicas=0
for _ in $(seq 1 90); do
  ready=$(curl -fsS --get --data-urlencode \
    'query=sum(pool_ready_steps{pool="g247",namespace="sagat-g247"})' \
    "http://127.0.0.1:$prometheus_port/api/v1/query" | jq -r '.data.result[0].value[1] // "0"' | awk '{printf "%d", $1}')
  replicas=$(kubectl --namespace sagat-g247 get deployment sagat-worker-g247 \
    -o jsonpath='{.status.readyReplicas}' 2>/dev/null || true)
  replicas=${replicas:-0}
  (( ready > peak_ready )) && peak_ready=$ready
  (( replicas > peak_replicas )) && peak_replicas=$replicas
  if (( peak_ready >= 5 && peak_replicas >= 2 )); then
    break
  fi
  sleep 2
done
if (( peak_ready < 5 )); then
  printf 'real pool_ready_steps never showed the persisted backlog (peak %d)\n' "$peak_ready" >&2
  exit 1
fi
if (( peak_replicas < 2 )); then
  printf 'KEDA never scaled the real worker deployment (peak %d)\n' "$peak_replicas" >&2
  exit 1
fi

cat <<'YAML' | kubectl apply --namespace sagat-g247 -f -
apiVersion: batch/v1
kind: Job
metadata: {name: g247-verify}
spec:
  backoffLimit: 1
  template:
    metadata: {labels: {app.kubernetes.io/component: validator}}
    spec:
      automountServiceAccountToken: false
      restartPolicy: Never
      securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
      containers:
        - name: verify
          image: sagat/g247-worker:local
          imagePullPolicy: Never
          command: [python]
          args: [/opt/sagat/scenario.py, verify]
          env:
            - {name: SAGAT_SERVER, value: "http://sagat-control-plane:8080/v1"}
            - {name: SAGAT_NAMESPACE, value: payments}
            - {name: G247_RUNS, value: "60"}
            - name: SAGAT_TOKEN
              valueFrom: {secretKeyRef: {name: g247-token, key: token}}
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            runAsNonRoot: true
            runAsUser: 10001
            capabilities: {drop: [ALL]}
YAML
kubectl wait --namespace sagat-g247 --for=condition=Complete job/g247-verify --timeout=300s

for _ in $(seq 1 240); do
  replicas=$(kubectl --namespace sagat-g247 get deployment sagat-worker-g247 \
    -o jsonpath='{.status.readyReplicas}' 2>/dev/null || true)
  replicas=${replicas:-0}
  ready=$(curl -fsS --get --data-urlencode \
    'query=sum(pool_ready_steps{pool="g247",namespace="sagat-g247"})' \
    "http://127.0.0.1:$prometheus_port/api/v1/query" | jq -r '.data.result[0].value[1] // "0"' | awk '{printf "%d", $1}')
  if (( replicas == 1 && ready == 0 )); then
    break
  fi
  sleep 2
done
if (( replicas != 1 || ready != 0 )); then
  printf 'worker pool did not recover to one replica with zero real queue metric (replicas=%d ready=%d)\n' \
    "$replicas" "$ready" >&2
  exit 1
fi

engine_pod=$(kubectl --namespace sagat-g247 get pod \
  -l app.kubernetes.io/component=execution-engine -o jsonpath='{.items[0].metadata.name}')
kubectl --namespace sagat-g247 delete pod "$engine_pod" --wait=false >/dev/null
kubectl wait --namespace sagat-g247 --for=condition=Available deployment/sagat-engine --timeout=180s

kubectl --namespace sagat-g247 get scaledobject sagat-worker-g247 -o json \
  | jq -e '.status.conditions[] | select(.type == "Ready" and .status == "True")' >/dev/null
kubectl --namespace sagat-g247 logs job/g247-enqueue | grep -F "G247_ENQUEUED=60" >/dev/null
kubectl --namespace sagat-g247 logs job/g247-verify | grep -F "G247_DRAINED=60" >/dev/null
kubectl --namespace sagat-g247 logs job/g247-network-denied \
  | grep -F "G247_UNDECLARED_API_DENIED=1" >/dev/null

printf 'G247 Kubernetes validation passed: peak_ready=%d peak_worker_replicas=%d recovered_replicas=%d\n' \
  "$peak_ready" "$peak_replicas" "$replicas"
