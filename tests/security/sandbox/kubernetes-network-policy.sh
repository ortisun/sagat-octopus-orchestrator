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
values_file=${NETWORK_POLICY_VALUES:-tests/security/sandbox/network-policy-values.yaml}
if [ ! -f "$values_file" ]; then
  printf 'network-policy test values not found at %s\n' "$values_file" >&2
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
--set engine.replicas=1"

expect_render_failure() {
  check_name=$1
  expected_message=$2
  shift 2
  set +e
  # shellcheck disable=SC2086
  render_error=$(helm template "$release" "$chart_dir" --namespace "$namespace" \
    -f "$values_file" $helm_args "$@" 2>&1)
  render_status=$?
  set -e
  if [ "$render_status" -eq 0 ]; then
    fail "$check_name was accepted"
  elif ! printf '%s' "$render_error" | grep -F "$expected_message" >/dev/null; then
    fail "$check_name failed without the expected validation: $render_error"
  fi
}

# A peer without a selector is a global allow, and an omitted port is every port. Reject unsafe
# values during Helm rendering, before Kubernetes can admit a broader policy than the operator meant.
expect_render_failure 'empty PostgreSQL peer' \
  'empty peer allows every destination' \
  --set-json 'networkPolicy.egress.postgres.to=[{}]'
expect_render_failure 'zero OIDC port' \
  'networkPolicy.egress.oidc.port must be a whole-number port between 1 and 65535' \
  --set networkPolicy.egress.oidc.port=0
expect_render_failure 'configured NATS without a destination' \
  'networkPolicy.egress.nats.to must contain at least one explicit peer' \
  --set-json 'networkPolicy.egress.nats.to=[]'
expect_render_failure 'extra egress without a port' \
  'networkPolicy.egress.controlPlaneExtra[0].ports must contain at least one explicit port' \
  --set-json 'networkPolicy.egress.controlPlaneExtra=[{"to":[{"ipBlock":{"cidr":"192.0.2.0/24"}}]}]'
expect_render_failure 'empty API ingress source' \
  'networkPolicy.apiFrom must contain at least one explicit peer' \
  --set-json 'networkPolicy.apiFrom=[]'
expect_render_failure 'global PostgreSQL CIDR' \
  'networkPolicy.egress.postgres.to[0].ipBlock.cidr cannot allow the global address space' \
  --set-json 'networkPolicy.egress.postgres.to=[{"ipBlock":{"cidr":"0.0.0.0/0"}}]'
expect_render_failure 'compressed global IPv6 PostgreSQL CIDR' \
  'networkPolicy.egress.postgres.to[0].ipBlock.cidr cannot allow the global address space' \
  --set-json 'networkPolicy.egress.postgres.to=[{"ipBlock":{"cidr":"::/0"}}]'
expect_render_failure 'expanded global IPv6 PostgreSQL CIDR' \
  'networkPolicy.egress.postgres.to[0].ipBlock.cidr cannot allow the global address space' \
  --set-json 'networkPolicy.egress.postgres.to=[{"ipBlock":{"cidr":"0:0:0:0:0:0:0:0/0"}}]'
expect_render_failure 'nonzero-address zero-prefix PostgreSQL CIDR' \
  'networkPolicy.egress.postgres.to[0].ipBlock.cidr cannot allow the global address space' \
  --set-json 'networkPolicy.egress.postgres.to=[{"ipBlock":{"cidr":"2001:db8::/0"}}]'
expect_render_failure 'empty metrics peer' \
  'networkPolicy.metricsFrom[0] cannot be empty because an empty peer allows every destination' \
  --set-json 'networkPolicy.metricsFrom=[{}]'
expect_render_failure 'global metrics namespace selector' \
  'networkPolicy.metricsFrom[0].namespaceSelector cannot be empty because an empty selector matches globally' \
  --set-json 'networkPolicy.metricsFrom=[{"namespaceSelector":{}}]'

# Nonzero IPv4 and IPv6 CIDRs remain valid explicit peers.
# shellcheck disable=SC2086
helm template "$release" "$chart_dir" --namespace "$namespace" -f "$values_file" $helm_args \
  --set-json 'networkPolicy.egress.postgres.to=[{"ipBlock":{"cidr":"192.0.2.0/24"}}]' \
  >"$work_dir/nonzero-ipv4.yaml"
# shellcheck disable=SC2086
helm template "$release" "$chart_dir" --namespace "$namespace" -f "$values_file" $helm_args \
  --set-json 'networkPolicy.egress.postgres.to=[{"ipBlock":{"cidr":"2001:db8::/64"}}]' \
  >"$work_dir/nonzero-ipv6.yaml"

# Disabling NetworkPolicy must suppress every policy, while an unused NATS configuration must not
# create a NATS egress rule or require a destination.
# shellcheck disable=SC2086
helm template "$release" "$chart_dir" --namespace "$namespace" -f "$values_file" $helm_args \
  --set networkPolicy.enabled=false >"$work_dir/disabled.yaml"
if grep -F 'kind: NetworkPolicy' "$work_dir/disabled.yaml" >/dev/null; then
  fail 'networkPolicy.enabled=false still rendered a NetworkPolicy'
fi
# shellcheck disable=SC2086
helm template "$release" "$chart_dir" --namespace "$namespace" -f "$values_file" $helm_args \
  --set-string nats.url= --show-only templates/networkpolicy-egress.yaml \
  >"$work_dir/nats-disabled.yaml"
if grep -F 'port: 14222' "$work_dir/nats-disabled.yaml" >/dev/null; then
  fail 'an empty nats.url still rendered the NATS egress destination'
fi

# Deliberately expand the fixed arguments one per line; no value comes from an untrusted source.
# shellcheck disable=SC2086
helm template "$release" "$chart_dir" --namespace "$namespace" -f "$values_file" $helm_args \
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
kubectl label namespace "$canary_namespace" sagat.io/security-canary=true --overwrite >/dev/null

# Prove the cluster actually enforces restricted Pod Security. Workload-template admission can emit
# only a warning, so a read-only root filesystem alone is not enough evidence.
cat >"$work_dir/privileged-counterexample.yaml" <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: privileged-counterexample
  namespace: $namespace
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: probe
      image: busybox:1.36
      command: [sh, -c, 'true']
      securityContext:
        privileged: true
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 65534
        capabilities: {drop: [ALL]}
        seccompProfile: {type: RuntimeDefault}
EOF
if privileged_result=$(kubectl create --dry-run=server \
  -f "$work_dir/privileged-counterexample.yaml" 2>&1); then
  printf 'restricted Pod Security admitted the privileged counterexample\n' >&2
  exit 2
fi
case $privileged_result in
  *'violates PodSecurity'*) ;;
  *)
    printf 'privileged counterexample failed for an unrelated reason: %s\n' \
      "$privileged_result" >&2
    exit 2
    ;;
esac

# Rendered workload resources are still server-validated for their own schemas. The restricted
# security decision is made below against real Pod resources derived from their complete templates.
if ! kubectl apply --dry-run=server --namespace "$namespace" \
  -f "$work_dir/rendered.yaml" -o json >"$work_dir/rendered.json"; then
  fail 'the chart does not pass server-side resource validation'
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

# Pod creation admission resolves the template's service account. Install only the rendered service
# accounts in this disposable namespace; the workload PodSpecs remain byte-for-byte equivalent.
jq '{apiVersion: "v1", kind: "List", items: [.items[] | select(.kind == "ServiceAccount")]}' \
  "$work_dir/rendered.json" >"$work_dir/serviceaccounts.json"
if [ "$(jq '.items | length' "$work_dir/serviceaccounts.json")" -gt 0 ]; then
  kubectl apply --namespace "$namespace" -f "$work_dir/serviceaccounts.json" >/dev/null
fi

jq --arg namespace "$namespace" '
  def template:
    if .kind == "CronJob" then .spec.jobTemplate.spec.template
    elif (.kind == "Deployment" or .kind == "StatefulSet" or .kind == "DaemonSet"
      or .kind == "Job" or .kind == "ReplicaSet") then .spec.template
    else empty
    end;
  [.items[] | template] as $templates
  | {
      apiVersion: "v1",
      kind: "List",
      items: [
        $templates
        | to_entries[]
        | {
            apiVersion: "v1",
            kind: "Pod",
            metadata: {
              name: "chart-template-psa-\(.key)",
              namespace: $namespace,
              labels: (.value.metadata.labels // {}),
              annotations: (.value.metadata.annotations // {})
            },
            spec: .value.spec
          }
      ]
    }
' "$work_dir/rendered.json" >"$work_dir/workload-pods.json"
if [ "$(jq '.items | length' "$work_dir/workload-pods.json")" -eq 0 ]; then
  fail 'the chart rendered no workload Pod templates'
  exit 1
fi
if ! kubectl create --dry-run=server -f "$work_dir/workload-pods.json" >/dev/null; then
  fail 'a rendered workload Pod violates the restricted Pod Security profile'
  exit 1
fi

# Extract every policy from the complete render. Feature 009 adds egress in a separate template after
# the Feature 010 chart integrates; naming one template here would silently leave those policies out.
jq '{apiVersion: "v1", kind: "List", items: [.items[] | select(.kind == "NetworkPolicy")]}' \
  "$work_dir/rendered.json" >"$work_dir/networkpolicies.json"
if [ "$(jq '.items | length' "$work_dir/networkpolicies.json")" -eq 0 ]; then
  fail 'the chart rendered no NetworkPolicy objects'
  exit 1
fi
if ! jq -e '
  [.items[]
    | select((.spec.policyTypes // []) | index("Egress"))
    | .metadata.name] | sort
  == [
    "sagat-octopus-orchestrator-control-plane-egress",
    "sagat-octopus-orchestrator-engine-egress",
    "sagat-octopus-orchestrator-worker-publish",
    "sagat-octopus-orchestrator-worker-render"
  ]
' "$work_dir/networkpolicies.json" >/dev/null; then
  fail 'the chart must render one explicit egress policy per control-plane, engine and worker role'
fi
if ! jq -e '
  def explicit_selector:
    type == "object"
      and (((.matchLabels // {}) | length) > 0
        or ((.matchExpressions // []) | length) > 0);
  def explicit_peer:
    type == "object"
      and length > 0
      and (if has("ipBlock") then
        (.ipBlock.cidr | type == "string" and length > 0)
          and (has("podSelector") or has("namespaceSelector") | not)
      else
        (has("podSelector") or has("namespaceSelector"))
          and (if has("podSelector") then (.podSelector | explicit_selector) else true end)
          and (if has("namespaceSelector") then
            (.namespaceSelector | explicit_selector) else true end)
      end);
  all(.items[];
    all(.spec.egress[]?;
      ((.ports // []) | length) > 0
        and ((.to // []) | length) > 0
        and all(.to[]; explicit_peer)))
' "$work_dir/networkpolicies.json" >/dev/null; then
  fail 'a rendered egress rule has an empty port, peer or global selector'
fi
if ! jq -e '
  any(.items[];
    .metadata.name == "sagat-octopus-orchestrator-worker-render"
      and ((.spec.policyTypes // []) | index("Ingress"))
      and ((.spec.ingress // []) == []))
' "$work_dir/networkpolicies.json" >/dev/null; then
  fail 'the worker policy must deny ingress by default'
fi
if ! jq -e '
  any(.items[];
    .metadata.name == "sagat-octopus-orchestrator-worker-publish"
      and ((.spec.ingress // []) | length) == 1)
' "$work_dir/networkpolicies.json" >/dev/null; then
  fail 'the publish worker must render its pool-specific ingress override'
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
      command: [sh, -c, 'while true; do nc -l -p 8080 </dev/null >/dev/null 2>&1 || true; done']
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
      command: [sh, -c, 'while true; do nc -l -p 9000 </dev/null >/dev/null 2>&1 || true; done']
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
      command: [sh, -c, 'while true; do nc -l -p 10080 </dev/null >/dev/null 2>&1 || true; done']
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
  name: worker-publish-probe
  namespace: $namespace
  labels:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: worker-publish
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: probe
      image: busybox:1.36
      command: [sh, -c, 'while true; do nc -l -p 10081 </dev/null >/dev/null 2>&1 || true; done']
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
  name: wrong-role-probe
  namespace: $namespace
  labels:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: intruder
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: probe
      image: busybox:1.36
      command: [sh, -c, 'while true; do sleep 3600; done']
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
      command: [sh, -c, 'while true; do sleep 3600; done']
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 65534
        capabilities: {drop: [ALL]}
        seccompProfile: {type: RuntimeDefault}
---
apiVersion: v1
kind: Service
metadata: {name: control-plane-probe, namespace: $namespace}
spec:
  selector:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: control-plane
  ports: [{name: api, port: 80, targetPort: 8080, protocol: TCP}]
---
apiVersion: v1
kind: Service
metadata: {name: execution-engine-probe, namespace: $namespace}
spec:
  selector:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: execution-engine
  ports: [{name: grpc, port: 9000, targetPort: 9000, protocol: TCP}]
---
apiVersion: v1
kind: Service
metadata: {name: worker-render-probe, namespace: $namespace}
spec:
  selector:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: worker-render
  ports: [{name: probe, port: 10080, targetPort: 10080, protocol: TCP}]
---
apiVersion: v1
kind: Service
metadata: {name: worker-publish-probe, namespace: $namespace}
spec:
  selector:
    app.kubernetes.io/name: sagat-octopus-orchestrator
    app.kubernetes.io/instance: $release
    app.kubernetes.io/component: worker-publish
  ports: [{name: probe, port: 10081, targetPort: 10081, protocol: TCP}]
EOF

: >"$work_dir/dependencies.yaml"
for dependency_spec in postgres:15432 oidc:18443 nats:14222 pool-extra:15555; do
  dependency=${dependency_spec%:*}
  dependency_port=${dependency_spec#*:}
  cat >>"$work_dir/dependencies.yaml" <<EOF
---
apiVersion: v1
kind: Pod
metadata:
  name: dependency-$dependency
  namespace: $canary_namespace
  labels: {sagat.io/dependency: $dependency}
spec:
  automountServiceAccountToken: false
  restartPolicy: Never
  securityContext: {runAsNonRoot: true, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: listener
      image: busybox:1.36
      command: [sh, -c, 'while true; do nc -l -p $dependency_port </dev/null >/dev/null 2>&1 || true; done']
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 65534
        capabilities: {drop: [ALL]}
        seccompProfile: {type: RuntimeDefault}
---
apiVersion: v1
kind: Service
metadata: {name: dependency-$dependency, namespace: $canary_namespace}
spec:
  selector: {sagat.io/dependency: $dependency}
  ports: [{name: tcp, port: $dependency_port, targetPort: $dependency_port, protocol: TCP}]
EOF
done

kubectl apply -f "$work_dir/probes.yaml" -f "$work_dir/dependencies.yaml" >/dev/null
for pod in control-plane-probe execution-engine-probe worker-render-probe \
  worker-publish-probe wrong-role-probe; do
  kubectl wait --namespace "$namespace" --for=condition=Ready "pod/$pod" --timeout=120s >/dev/null
done
kubectl wait --namespace "$canary_namespace" --for=condition=Ready \
  pod/unrestricted-canary --timeout=120s >/dev/null
for dependency in postgres oidc nats pool-extra; do
  kubectl wait --namespace "$canary_namespace" --for=condition=Ready \
    "pod/dependency-$dependency" --timeout=120s >/dev/null
done

api_ip=$(kubectl get service kubernetes --namespace default -o jsonpath='{.spec.clusterIP}')
if [ -z "$api_ip" ] || [ "$api_ip" = None ]; then
  printf 'the kubernetes service has no routable ClusterIP\n' >&2
  exit 2
fi
control_plane_ip=$(kubectl get service control-plane-probe --namespace "$namespace" \
  -o jsonpath='{.spec.clusterIP}')
engine_ip=$(kubectl get service execution-engine-probe --namespace "$namespace" \
  -o jsonpath='{.spec.clusterIP}')
worker_ip=$(kubectl get service worker-render-probe --namespace "$namespace" \
  -o jsonpath='{.spec.clusterIP}')
publish_worker_ip=$(kubectl get service worker-publish-probe --namespace "$namespace" \
  -o jsonpath='{.spec.clusterIP}')
postgres_ip=$(kubectl get service dependency-postgres --namespace "$canary_namespace" \
  -o jsonpath='{.spec.clusterIP}')
oidc_ip=$(kubectl get service dependency-oidc --namespace "$canary_namespace" \
  -o jsonpath='{.spec.clusterIP}')
nats_ip=$(kubectl get service dependency-nats --namespace "$canary_namespace" \
  -o jsonpath='{.spec.clusterIP}')
pool_extra_ip=$(kubectl get service dependency-pool-extra --namespace "$canary_namespace" \
  -o jsonpath='{.spec.clusterIP}')

expect_reachable() {
  description=$1
  source_namespace=$2
  source_pod=$3
  destination=$4
  port=$5
  attempt=0
  while [ "$attempt" -lt 10 ]; do
    if kubectl exec --namespace "$source_namespace" "$source_pod" -- \
      nc -z -w 2 "$destination" "$port" >/dev/null 2>&1; then
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 1
  done
  fail "$description was not reachable at $destination:$port"
  return 1
}

expect_blocked() {
  description=$1
  source_namespace=$2
  source_pod=$3
  destination=$4
  port=$5
  set +e
  # The positional parameters are intentionally expanded by the shell inside the probe container.
  # shellcheck disable=SC2016
  probe_output=$(kubectl exec --namespace "$source_namespace" "$source_pod" -- \
    sh -c 'nc -z -w 3 "$1" "$2"; printf "__SAGAT_NC_STATUS__:%s\n" "$?"' \
    sh "$destination" "$port" 2>&1)
  exec_status=$?
  set -e
  if [ "$exec_status" -ne 0 ]; then
    printf 'network denial probe could not execute in %s/%s: %s\n' \
      "$source_namespace" "$source_pod" "$probe_output" >&2
    exit 2
  fi
  case $probe_output in
    *'__SAGAT_NC_STATUS__:0'*)
      fail "$description was reachable at $destination:$port"
      ;;
    *'__SAGAT_NC_STATUS__:1') ;;
    *)
      printf 'network denial probe returned an invalid nc status in %s/%s: %s\n' \
        "$source_namespace" "$source_pod" "$probe_output" >&2
      exit 2
      ;;
  esac
}

# A missing or terminated source must invalidate the test rather than look like a denied packet.
set +e
(expect_blocked 'infrastructure failure counterexample' \
  "$namespace" missing-denial-source "$api_ip" 443) \
  >"$work_dir/infrastructure-failure.log" 2>&1
infrastructure_failure_status=$?
set -e
if [ "$infrastructure_failure_status" -ne 2 ] || \
  ! grep -F 'network denial probe could not execute' \
    "$work_dir/infrastructure-failure.log" >/dev/null; then
  fail 'a missing denial-probe source was not classified as an infrastructure failure'
fi

# Establish that this cluster and probe image can reach the destination without a policy. Otherwise
# a failed connection below could be a cluster-level block rather than evidence about the chart.
if ! expect_reachable 'unrestricted canary to Kubernetes API' \
  "$canary_namespace" unrestricted-canary "$api_ip" 443; then
  printf 'the unrestricted connectivity control failed\n' >&2
  exit 2
fi
for dependency_spec in postgres:$postgres_ip:15432 oidc:$oidc_ip:18443 nats:$nats_ip:14222 \
  pool-extra:$pool_extra_ip:15555; do
  dependency=${dependency_spec%%:*}
  destination_and_port=${dependency_spec#*:}
  destination=${destination_and_port%:*}
  port=${destination_and_port#*:}
  if ! expect_reachable "unrestricted canary to $dependency" \
    "$canary_namespace" unrestricted-canary "$destination" "$port"; then
    printf 'the %s dependency connectivity control failed\n' "$dependency" >&2
    exit 2
  fi
  if ! expect_reachable "same-namespace wrong-role control to $dependency" \
    "$namespace" wrong-role-probe "$destination" "$port"; then
    printf 'the wrong-role source connectivity control failed\n' >&2
    exit 2
  fi
done

# Prove each listener used by a deny assertion is running. Loopback is not a declared external
# destination and remains inside the worker's own network namespace.
if ! expect_reachable 'worker listener loopback control' \
  "$namespace" worker-render-probe 127.0.0.1 10080; then
  printf 'the worker listener control failed\n' >&2
  exit 2
fi

# The Kubernetes API is never a declared dependency of these workloads (service-account tokens are
# disabled). Use its captured ClusterIP so a DNS failure cannot masquerade as blocked egress.
for pod in control-plane-probe execution-engine-probe worker-render-probe worker-publish-probe; do
  if ! kubectl exec --namespace "$namespace" "$pod" -- \
    nslookup kubernetes.default.svc.cluster.local >/dev/null 2>&1; then
    fail "$pod cannot resolve cluster DNS, a required positive egress dependency"
  fi
  expect_blocked "$pod to undeclared Kubernetes API" "$namespace" "$pod" "$api_ip" 443
done

# Declared role dependencies remain reachable through Services. The control-plane service maps port
# 80 to container port 8080, proving the worker policy names the post-DNAT pod port correctly.
expect_reachable 'control plane to PostgreSQL' \
  "$namespace" control-plane-probe "$postgres_ip" 15432 || true
expect_reachable 'control plane to OIDC/JWKS' \
  "$namespace" control-plane-probe "$oidc_ip" 18443 || true
expect_reachable 'engine to PostgreSQL' \
  "$namespace" execution-engine-probe "$postgres_ip" 15432 || true
expect_reachable 'engine to OIDC/JWKS' \
  "$namespace" execution-engine-probe "$oidc_ip" 18443 || true
expect_reachable 'engine to NATS' \
  "$namespace" execution-engine-probe "$nats_ip" 14222 || true
expect_reachable 'worker to control-plane Service 80 -> pod 8080' \
  "$namespace" worker-render-probe "$control_plane_ip" 80 || true
expect_reachable 'worker to engine gRPC' \
  "$namespace" worker-render-probe "$engine_ip" 9000 || true
expect_reachable 'publish worker to control-plane API' \
  "$namespace" worker-publish-probe "$control_plane_ip" 80 || true
expect_reachable 'publish worker to engine gRPC' \
  "$namespace" worker-publish-probe "$engine_ip" 9000 || true
expect_reachable 'publish worker pool-specific destination' \
  "$namespace" worker-publish-probe "$pool_extra_ip" 15555 || true

# The role matrix excludes NATS from the control plane and every external dependency from workers.
expect_blocked 'control plane to undeclared NATS' \
  "$namespace" control-plane-probe "$nats_ip" 14222
expect_blocked 'worker to undeclared PostgreSQL' \
  "$namespace" worker-render-probe "$postgres_ip" 15432
expect_blocked 'worker to undeclared OIDC/JWKS' \
  "$namespace" worker-render-probe "$oidc_ip" 18443
expect_blocked 'worker to undeclared NATS' \
  "$namespace" worker-render-probe "$nats_ip" 14222
expect_blocked 'render worker to publish pool-specific destination' \
  "$namespace" worker-render-probe "$pool_extra_ip" 15555

# Existing ingress policies are narrowed by explicit peers in the fixture. The same worker labels in
# another namespace and a different role in this namespace must not inherit worker access.
expect_blocked 'same worker labels from another namespace to control plane' \
  "$canary_namespace" unrestricted-canary "$control_plane_ip" 80
expect_blocked 'same worker labels from another namespace to engine' \
  "$canary_namespace" unrestricted-canary "$engine_ip" 9000
expect_blocked 'wrong role in the release namespace to control plane' \
  "$namespace" wrong-role-probe "$control_plane_ip" 80
expect_blocked 'wrong role in the release namespace to engine' \
  "$namespace" wrong-role-probe "$engine_ip" 9000
expect_blocked 'worker default-deny ingress' \
  "$canary_namespace" unrestricted-canary "$worker_ip" 10080
expect_reachable 'publish worker pool-specific ingress source' \
  "$namespace" wrong-role-probe "$publish_worker_ip" 10081 || true
expect_blocked 'publish worker ingress from another namespace' \
  "$canary_namespace" unrestricted-canary "$publish_worker_ip" 10081

policies=$(kubectl get networkpolicy --namespace "$namespace" -o json)
for component in control-plane execution-engine worker-render worker-publish; do
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
printf 'Kubernetes pod security, ingress isolation and role-specific egress checks passed\n'
