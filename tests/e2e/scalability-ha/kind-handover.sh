#!/usr/bin/env bash
# The Job of the automatic handover of the cron triggers in a real Kubernetes (T018-088, OD-2026-11-15-14, G2250, G2289, ADR-0091
# decision 1). A kind cluster of its own, the images of this tree loaded into it, a PostgreSQL in the cluster migrated by
# `control-plane migrate` (which creates sagat_handover and sets its login), then the chart of infra/helm installed with the job on:
#
#   1. revision 1: the Job's first round is written under its run id, the UID of the Job (the controller-uid label, D2263);
#   2. `helm upgrade --atomic` to a revision whose engine cannot start: Helm deletes the Job of revision 1 (its pod receives
#      SIGTERM and writes INTERRUPTED), the upgrade times out and is rolled back, which deletes the Job of revision 2 and creates
#      the Job of revision 1 again under a new UID, so a new run with a first round of its own (G2250, G2289);
#   3. the first round of that run is moved back by a horizon as the owner (the tests and the compose smoke test do the same), and
#      the Job hands over: complete, the state HANDED_OVER with the mode AUTOMATIC and the actor of that run.
#
#   tests/e2e/scalability-ha/kind-handover.sh
#   SAGAT_KIND_KEEP=1 tests/e2e/scalability-ha/kind-handover.sh     # leave the cluster for a look (kind delete cluster --name ...)
#
# Needs Docker, kind, kubectl and helm. The CI runs it only for a release (the job "handover job in a kind cluster"). The cluster,
# its name and the images it builds are its own; it deletes the cluster when it ends.
set -euo pipefail
cd "$(dirname "$0")/../../.."

# kind v0.33.0's node image for Kubernetes 1.37, pinned by digest; the PostgreSQL 16 image the benchmarks pin.
NODE_IMAGE="${SAGAT_KIND_NODE_IMAGE:-kindest/node:v1.37.0@sha256:a1ed56cfb0e7b93589bdf97c8cd566405a265939e3620fc4f5de89adff580ae5}"
PG_IMAGE="${SAGAT_KIND_PG_IMAGE:-postgres@sha256:cf78e76683b9ca8c5733cbbdce6c9262b45b6767934dd0a95e671f9a0fc20685}"
CLUSTER="sagat-handover-$$"
NS=sagat
TAG=kind
CP_IMAGE="sagat-octopus-orchestrator/control-plane:$TAG"
ENGINE_IMAGE="sagat-octopus-orchestrator/execution-engine:$TAG"
WORK=$(mktemp -d)
KUBECONFIG="$WORK/kubeconfig"
export KUBECONFIG

cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    echo "--- the kind handover test FAILED (exit $status); what the cluster shows:" >&2
    kubectl -n "$NS" get jobs,pods -o wide >&2 || true
    kubectl -n "$NS" logs -l app.kubernetes.io/component=schedule-trigger-handover --tail=30 --prefix >&2 || true
    helm -n "$NS" history sagat >&2 || true
  fi
  if [ "${SAGAT_KIND_KEEP:-0}" != 1 ]; then kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true; fi
  rm -rf "$WORK"
  exit "$status"
}
trap cleanup EXIT

fail() { echo "$*" >&2; exit 1; }
password() { python3 -c 'import secrets; print(secrets.token_urlsafe(24))'; }
until_true() {  # until_true <seconds> <description> <command...>
  local seconds=$1 description=$2; shift 2
  for _ in $(seq 1 "$seconds"); do
    if "$@" >/dev/null 2>&1; then echo "ok: $description"; return 0; fi
    sleep 1
  done
  fail "timed out waiting for: $description"
}

echo "--- a kind cluster of its own ($CLUSTER, $NODE_IMAGE)"
kind create cluster --name "$CLUSTER" --image "$NODE_IMAGE" --kubeconfig "$KUBECONFIG" --wait 180s

echo "--- the images of this tree"
if [ "${SAGAT_KIND_SKIP_BUILD:-0}" != 1 ]; then
  docker build -q -f infra/docker/Dockerfile --target control-plane -t "$CP_IMAGE" . >/dev/null
  docker build -q -f infra/docker/Dockerfile --target execution-engine -t "$ENGINE_IMAGE" . >/dev/null
fi
kind load docker-image "$CP_IMAGE" "$ENGINE_IMAGE" --name "$CLUSTER"

OWNER_PASSWORD=$(password)
APP_PASSWORD=$(password)
HANDOVER_PASSWORD=$(password)
kubectl create namespace "$NS" >/dev/null
kubectl -n "$NS" create secret generic postgres \
  --from-literal=POSTGRES_PASSWORD="$OWNER_PASSWORD" --from-literal=SAGAT_APP_PASSWORD="$APP_PASSWORD" >/dev/null
kubectl -n "$NS" create configmap postgres-init --from-file=infra/docker/postgres-init/ >/dev/null

echo "--- PostgreSQL in the cluster"
kubectl -n "$NS" apply -f - >/dev/null <<YAML
apiVersion: apps/v1
kind: Deployment
metadata: {name: postgres}
spec:
  replicas: 1
  selector: {matchLabels: {app: postgres}}
  template:
    metadata: {labels: {app: postgres}}
    spec:
      containers:
        - name: postgres
          image: $PG_IMAGE
          env:
            - {name: POSTGRES_USER, value: sagat}
            - {name: POSTGRES_DB, value: sagat}
          envFrom: [{secretRef: {name: postgres}}]
          ports: [{containerPort: 5432}]
          readinessProbe: {exec: {command: [pg_isready, -U, sagat, -d, sagat]}, periodSeconds: 2}
          volumeMounts: [{name: init, mountPath: /docker-entrypoint-initdb.d, readOnly: true}]
      volumes: [{name: init, configMap: {name: postgres-init}}]
---
apiVersion: v1
kind: Service
metadata: {name: postgres}
spec:
  selector: {app: postgres}
  ports: [{port: 5432, targetPort: 5432}]
YAML
kubectl -n "$NS" rollout status deployment/postgres --timeout=300s
owner_sql() { kubectl -n "$NS" exec deploy/postgres -- psql -U sagat -d sagat -v ON_ERROR_STOP=1 -tAc "$1"; }
until_true 60 "PostgreSQL answers the owner" owner_sql "SELECT 1"

echo "--- migrate, with the password of sagat_handover"
kubectl -n "$NS" run migrate --image="$CP_IMAGE" --image-pull-policy=IfNotPresent --restart=Never \
  --env="DATABASE_URL=postgres://sagat:$OWNER_PASSWORD@postgres:5432/sagat" --env=SAGAT_ENV=dev \
  --env="SAGAT_HANDOVER_PASSWORD=$HANDOVER_PASSWORD" --env=SAGAT_HANDOVER_MIN_LIVE=1 -- migrate >/dev/null
kubectl -n "$NS" wait --for=jsonpath='{.status.phase}'=Succeeded pod/migrate --timeout=300s >/dev/null \
  || { kubectl -n "$NS" logs pod/migrate >&2; fail "migrate did not succeed"; }
[ "$(owner_sql "SELECT rolcanlogin FROM pg_roles WHERE rolname = 'sagat_handover'")" = t ] || fail "migrate gave sagat_handover no login"
echo "ok: migrated; sagat_handover may log in"

kubectl -n "$NS" create secret generic sagat-db \
  --from-literal=DATABASE_URL="postgres://sagat_app:$APP_PASSWORD@postgres:5432/sagat" >/dev/null
kubectl -n "$NS" create secret generic sagat-handover \
  --from-literal=DATABASE_URL="postgres://sagat_handover@postgres:5432/sagat" \
  --from-literal=SAGAT_HANDOVER_PASSWORD="$HANDOVER_PASSWORD" >/dev/null
unset OWNER_PASSWORD APP_PASSWORD HANDOVER_PASSWORD

cat > "$WORK/values.yaml" <<'YAML'
environment: dev
image: {tag: kind, pullPolicy: IfNotPresent}
database: {existingSecret: sagat-db, maxConnections: 5}
oidc:
  issuer: http://idp.invalid/
  audience: sagat
  jwksUri: http://idp.invalid/jwks.json
  allowInsecureJwksHttp: true
eventTransport: none
schedules: {autoHandover: {databaseSecret: sagat-handover}}
controlPlane:
  replicas: 1
  resources: {requests: {cpu: 50m, memory: 128Mi}, limits: {memory: 512Mi}}
engine:
  replicas: 1
  resources: {requests: {cpu: 50m, memory: 128Mi}, limits: {memory: 768Mi}}
networkPolicy: {enabled: false}
YAML

job_of() {  # job_of <revision>: the name of the Job of that revision, or nothing
  kubectl -n "$NS" get jobs -l app.kubernetes.io/component=schedule-trigger-handover \
    -o jsonpath="{range .items[*]}{.metadata.name}{\"\\n\"}{end}" | grep -E -- "-schedule-trigger-handover-$1(-[0-9a-f]{10})?\$" || true
}
uid_of() { kubectl -n "$NS" get job "$1" -o jsonpath='{.metadata.uid}'; }
reason_of() { owner_sql "SELECT coalesce(last_reason, '') FROM schedule_trigger_handover_attempts WHERE run_id = '$1'"; }
reason_is() { [ "$(reason_of "$1")" = "$2" ]; }

echo "--- revision 1: the chart with the job on"
helm install sagat infra/helm -n "$NS" -f "$WORK/values.yaml" --wait --timeout 5m >/dev/null
FIRST=$(job_of 1)
[ -n "$FIRST" ] || fail "revision 1 has no handover Job"
RUN1=$(uid_of "$FIRST")
until_true 180 "the Job of revision 1 writes its first round under its UID ($RUN1)" reason_is "$RUN1" FIRST_HORIZON

echo "--- revision 2 cannot start an engine: helm upgrade --atomic rolls it back"
if helm upgrade sagat infra/helm -n "$NS" -f "$WORK/values.yaml" \
     --set image.executionEngine=sagat-octopus-orchestrator/no-such-engine --atomic --timeout 90s >/dev/null 2>&1; then
  fail "an upgrade whose engine cannot start was not refused"
fi
helm -n "$NS" history sagat -o json | python3 -c '
import json, sys
history = json.load(sys.stdin)
states = [(h["revision"], h["status"]) for h in history]
assert states[-1][1] == "deployed" and any(status == "failed" for _, status in states), states
assert any("Rollback" in h.get("description", "") for h in history), history
'
echo "ok: revision 2 failed and was rolled back"
until_true 60 "the Job of revision 2 is gone" bash -c "[ -z \"\$(kubectl -n $NS get jobs -o name | grep -E -- '-schedule-trigger-handover-2(-|\$)')\" ]"
until_true 60 "the pod of revision 1's Job received SIGTERM when Helm deleted it, and wrote INTERRUPTED" reason_is "$RUN1" INTERRUPTED
AGAIN=$(job_of 1)
[ -n "$AGAIN" ] || fail "the rollback did not create the Job of revision 1 again"
RUN3=$(uid_of "$AGAIN")
[ "$RUN3" != "$RUN1" ] || fail "the Job the rollback recreated kept the UID $RUN1"
until_true 180 "the recreated Job starts a run of its own ($RUN3)" reason_is "$RUN3" FIRST_HORIZON
FIRST1=$(owner_sql "SELECT first_round_at FROM schedule_trigger_handover_attempts WHERE run_id = '$RUN1'")
FIRST3=$(owner_sql "SELECT first_round_at FROM schedule_trigger_handover_attempts WHERE run_id = '$RUN3'")
[ "$(owner_sql "SELECT '$FIRST3'::timestamptz > '$FIRST1'::timestamptz")" = t ] || fail "the new run kept the old first round"
echo "ok: a new run, a first round and a deadline of its own"

echo "--- the job hands over"
owner_sql "UPDATE schedule_trigger_handover_attempts SET first_round_at = first_round_at - horizon - interval '1 minute' WHERE run_id = '$RUN3'" >/dev/null
kubectl -n "$NS" wait --for=condition=complete "job/$AGAIN" --timeout=300s >/dev/null || fail "the Job did not complete"
HANDED=$(owner_sql "SELECT state || ' ' || changed_mode || ' ' || changed_by FROM schedule_trigger_handover")
[ "$HANDED" = "HANDED_OVER AUTOMATIC system:handover-job:$RUN3" ] || fail "after the Job the handover is '$HANDED'"
echo "ok: the Job completed; $HANDED"
echo "--- the kind handover test passed"
