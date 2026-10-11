#!/bin/sh
# Verifies T009-031 against a real Kubernetes network-policy implementation. The caller must pass
# an explicit disposable cluster: this script creates and removes only its two unique namespaces.
set -eu

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  failures=$((failures + 1))
}

for command_name in helm kubectl jq mktemp; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command_name" >&2
    exit 2
  }
done

if [ -z "${KUBECONFIG:-}" ]; then
  printf 'KUBECONFIG must name an explicit disposable test cluster\n' >&2
  exit 2
fi
if [ ! -f "$KUBECONFIG" ]; then
  printf 'KUBECONFIG does not exist: %s\n' "$KUBECONFIG" >&2
  exit 2
fi

chart_dir=${CHART_DIR:-infra/helm}
if [ ! -f "$chart_dir/Chart.yaml" ]; then
  printf 'chart not found at %s (set CHART_DIR after feature 010 is integrated)\n' "$chart_dir" >&2
  exit 2
fi

suffix="$(date +%s)-$$"
namespace="sagat-security-test-$suffix"
canary_namespace="sagat-security-canary-$suffix"
release=sagat-security
work_dir=$(mktemp -d)
failures=0
namespace_created=false
canary_namespace_created=false

cleanup() {
  if [ "$namespace_created" = true ]; then
    kubectl delete namespace "$namespace" --wait=false >/dev/null 2>&1 || true
  fi
  if [ "$canary_namespace_created" = true ]; then
    kubectl delete namespace "$canary_namespace" --wait=false >/dev/null 2>&1 || true
  fi
  rm -rf "$work_dir"
}
trap cleanup EXIT HUP INT TERM

for test_namespace in "$namespace" "$canary_namespace"; do
  if kubectl get namespace "$test_namespace" >/dev/null 2>&1; then
    printf 'refusing to reuse existing namespace: %s\n' "$test_namespace" >&2
    exit 2
  fi
done

helm_args="--set database.existingSecret=unused-runtime-secret
--set oidc.issuer=https://issuer.example.invalid
--set oidc.audience=sagat-security-test
--set oidc.jwksUri=https://issuer.example.invalid/.well-known/jwks.json
--set controlPlane.replicas=1
--set engine.replicas=1
--set workerPools[0].name=render
--set workerPools[0].image=busybox:1.36
--set workerPools[0].activities[0]=act.render"

# Deliberately expand the fixed arguments one per line; no value comes from an untrusted source.
# shellcheck disable=SC2086
helm template "$release" "$chart_dir" --namespace "$namespace" $helm_args \
  >"$work_dir/rendered.yaml"

kubectl create namespace "$namespace" >/dev/null
namespace_created=true
kubectl create namespace "$canary_namespace" >/dev/null
canary_namespace_created=true
for test_namespace in "$namespace" "$canary_namespace"; do
  kubectl label namespace "$test_namespace" \
    pod-security.kubernetes.io/enforce=restricted \
    pod-security.kubernetes.io/enforce-version=latest \
    --overwrite >/dev/null
done

# The API server's Pod Security admission evaluates every chart workload without starting the
# product images or requiring its external database and identity provider.
if ! kubectl apply --dry-run=server --namespace "$namespace" \
  -f "$work_dir/rendered.yaml" -o json >"$work_dir/rendered.json"; then
  fail 'the chart is not admitted by the restricted Pod Security profile'
  exit 1
elif ! jq -e '
  def podspec:
    if .kind == "CronJob" then .spec.jobTemplate.spec.template.spec
    elif (.kind == "Deployment" or .kind == "StatefulSet" or .kind == "DaemonSet"
      or .kind == "Job" or .kind == "ReplicaSet") then .spec.template.spec
    else empty
    end;
  [.items[] | podspec | ((.initContainers // []) + (.containers // []))[]]
    | length > 0
      and all(.securityContext.readOnlyRootFilesystem == true)
' "$work_dir/rendered.json" >/dev/null; then
  fail 'every rendered workload container must set readOnlyRootFilesystem=true'
fi

# Extract every policy from the complete render. Feature 009 adds egress in a separate template after
# the Feature 010 chart integrates; naming one template here would silently leave those policies out.
jq '{apiVersion: "v1", kind: "List", items: [.items[] | select(.kind == "NetworkPolicy")]}' \
  "$work_dir/rendered.json" >"$work_dir/networkpolicies.json"
if [ "$(jq '.items | length' "$work_dir/networkpolicies.json")" -eq 0 ]; then
  fail 'the chart rendered no NetworkPolicy objects'
  exit 1
fi
kubectl apply --namespace "$namespace" -f "$work_dir/networkpolicies.json" >/dev/null

cat >"$work_dir/probes.yaml" <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: control-plane-probe
  namespace: $namespace
  labels:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: control-plane
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: probe
      image: busybox:1.36
      command: [sh, -c, 'sleep 300']
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 65534
        capabilities: {drop: [ALL]}
        seccompProfile: {type: RuntimeDefault}
---
apiVersion: v1
kind: Pod
metadata:
  name: execution-engine-probe
  namespace: $namespace
  labels:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: execution-engine
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: probe
      image: busybox:1.36
      command: [sh, -c, 'sleep 300']
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 65534
        capabilities: {drop: [ALL]}
        seccompProfile: {type: RuntimeDefault}
---
apiVersion: v1
kind: Pod
metadata:
  name: worker-render-probe
  namespace: $namespace
  labels:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: worker-render
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: probe
      image: busybox:1.36
      command: [sh, -c, 'sleep 300']
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 65534
        capabilities: {drop: [ALL]}
        seccompProfile: {type: RuntimeDefault}
---
apiVersion: v1
kind: Pod
metadata:
  name: unrestricted-canary
  namespace: $canary_namespace
  labels:
    app.kubernetes.io/name: sagat-security-canary
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: probe
      image: busybox:1.36
      command: [sh, -c, 'sleep 300']
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 65534
        capabilities: {drop: [ALL]}
        seccompProfile: {type: RuntimeDefault}
EOF

kubectl apply -f "$work_dir/probes.yaml" >/dev/null
for pod in control-plane-probe execution-engine-probe worker-render-probe; do
  kubectl wait --namespace "$namespace" --for=condition=Ready "pod/$pod" --timeout=120s >/dev/null
done
kubectl wait --namespace "$canary_namespace" --for=condition=Ready \
  pod/unrestricted-canary --timeout=120s >/dev/null

api_ip=$(kubectl get service kubernetes --namespace default -o jsonpath='{.spec.clusterIP}')
if [ -z "$api_ip" ] || [ "$api_ip" = None ]; then
  printf 'the kubernetes service has no routable ClusterIP\n' >&2
  exit 2
fi

# Establish that this cluster and probe image can reach the destination without a policy. Otherwise
# a failed connection below could be a cluster-level block rather than evidence about the chart.
if ! kubectl exec --namespace "$canary_namespace" unrestricted-canary -- \
  nc -z -w 3 "$api_ip" 443 >/dev/null 2>&1; then
  printf 'unrestricted canary could not reach Kubernetes API at %s\n' "$api_ip" >&2
  exit 2
fi

# The Kubernetes API is never a declared dependency of these workloads (service-account tokens are
# disabled). Use its captured ClusterIP so a DNS failure cannot masquerade as blocked egress.
for pod in control-plane-probe execution-engine-probe worker-render-probe; do
  if ! kubectl exec --namespace "$namespace" "$pod" -- \
    nslookup kubernetes.default.svc.cluster.local >/dev/null 2>&1; then
    fail "$pod cannot resolve cluster DNS, a required positive egress dependency"
  fi
  if kubectl exec --namespace "$namespace" "$pod" -- \
    nc -z -w 3 "$api_ip" 443 >/dev/null 2>&1; then
    fail "$pod reached the undeclared Kubernetes API destination"
  fi
done

policies=$(kubectl get networkpolicy --namespace "$namespace" -o json)
for component in control-plane execution-engine worker-render; do
  selected=$(printf '%s' "$policies" | jq --arg component "$component" '
    def requirement_matches($labels):
      . as $requirement
      | if $requirement.operator == "In" then
          (($labels[$requirement.key] // null) as $value
            | $value != null and (($requirement.values // []) | index($value) != null))
        elif $requirement.operator == "NotIn" then
          (($labels[$requirement.key] // null) as $value
            | $value == null or (($requirement.values // []) | index($value) == null))
        elif $requirement.operator == "Exists" then
          ($labels | has($requirement.key))
        elif $requirement.operator == "DoesNotExist" then
          (($labels | has($requirement.key)) | not)
        else false
        end;
    def selector_matches($labels):
      ((.matchLabels // {}) | to_entries
        | all(. as $label | $labels[$label.key] == $label.value))
      and ((.matchExpressions // []) | all(requirement_matches($labels)));
    {
      "app.kubernetes.io/name": "sagat-octopus-orchestrator",
      "app.kubernetes.io/instance": "sagat-security",
      "app.kubernetes.io/component": $component
    } as $labels
    |
    [.items[]
      | select(
          (((.spec.policyTypes // []) | index("Egress")) != null)
          or (.spec.egress != null))
      | select(.spec.podSelector | selector_matches($labels))]
    | length')
  if [ "$selected" -eq 0 ]; then
    fail "$component has no selecting Egress NetworkPolicy"
  fi
done

if [ "$failures" -ne 0 ]; then
  printf '%s Kubernetes security assertion(s) failed\n' "$failures" >&2
  exit 1
fi
printf 'Kubernetes pod security and default-deny egress checks passed\n'
