#!/usr/bin/env bash
# Smoke test of the local environment (US03): builds the image, starts the stack, checks that the
# API, PostgreSQL and NATS are healthy, that migrations and the seed are idempotent and that the API
# runs as the least-privilege database role, that the development identity provider signs a token the
# API accepts, that the Studio is served and, when SAGAT_BIN names a built `sagat`, that the first day
# works: doctor, a workflow published, served by `worker dev` and run to the end. Then it tears
# everything down.
#
#   scripts/smoke-compose.sh                 # uses infra/docker/.env.example
#   ENV_FILE=infra/docker/.env scripts/smoke-compose.sh
#   SAGAT_BIN=target/debug/sagat scripts/smoke-compose.sh     # also the first day with the CLI
#
# The stack runs under its own throw-away compose project name, so the final `down -v` can never
# touch the volumes of the development environment you start by hand.
set -euo pipefail

cd "$(dirname "$0")/.."
ENV_FILE="${ENV_FILE:-infra/docker/.env.example}"
PROJECT="sagat-smoke-$$"
COMPOSE=(docker compose -p "$PROJECT" -f infra/docker/compose.yaml --env-file "$ENV_FILE")

# The env file is data, not code: read single values instead of sourcing it. An exported variable
# of the same name wins, like it does for compose itself.
env_value() {  # env_value NAME [DEFAULT]
  local name=$1 default=${2:-} from_file
  from_file=$(grep -E "^${name}=" "$ENV_FILE" | tail -n1 | cut -d= -f2- || true)
  printf '%s' "${!name:-${from_file:-$default}}"
}

API="http://127.0.0.1:$(env_value SAGAT_API_PORT 18080)/v1"
NATS_MONITOR="http://127.0.0.1:$(env_value SAGAT_NATS_MONITOR_PORT 18222)"
DB_OWNER=$(env_value POSTGRES_USER sagat)
DB_NAME=$(env_value POSTGRES_DB sagat)
APP_PASSWORD=$(env_value SAGAT_APP_PASSWORD)

cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    echo "--- smoke test FAILED (exit $status); service state and recent logs:" >&2
    "${COMPOSE[@]}" ps -a >&2 || true
    "${COMPOSE[@]}" logs --tail=40 >&2 || true
  fi
  "${COMPOSE[@]}" --profile seed down -v --remove-orphans >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT

echo "--- building and starting the stack (project $PROJECT)"
"${COMPOSE[@]}" up -d --build

wait_for() {  # wait_for <description> <command...>
  local description=$1; shift
  for _ in $(seq 1 120); do
    if "$@" >/dev/null 2>&1; then echo "ok: $description"; return 0; fi
    sleep 1
  done
  echo "timed out waiting for: $description" >&2
  return 1
}

wait_for "API liveness  ($API/health)" curl -fsS "$API/health"
wait_for "API readiness ($API/ready, includes PostgreSQL)" curl -fsS "$API/ready"
wait_for "NATS JetStream monitor ($NATS_MONITOR/healthz)" curl -fsS "$NATS_MONITOR/healthz"

echo "--- migrations are idempotent (the migrate job already ran once; run it again)"
"${COMPOSE[@]}" run --rm migrate

echo "--- the development seed is idempotent"
"${COMPOSE[@]}" --profile seed run --rm seed
"${COMPOSE[@]}" --profile seed run --rm seed
TENANTS=$("${COMPOSE[@]}" exec -T postgres psql -U "$DB_OWNER" -d "$DB_NAME" -tAc "SELECT count(*) FROM tenants")
[ "$TENANTS" = "1" ] || { echo "expected exactly 1 seeded tenant, found $TENANTS" >&2; exit 1; }
echo "ok: exactly one tenant after seeding twice"

echo "--- the API serves requests as the least-privilege role"
API_USERS=$("${COMPOSE[@]}" exec -T postgres psql -U "$DB_OWNER" -d "$DB_NAME" -tAc \
  "SELECT string_agg(DISTINCT usename, ',') FROM pg_stat_activity WHERE application_name = 'sagat-octopus-orchestrator' AND datname = '$DB_NAME'")
[ "$API_USERS" = "sagat_app" ] || { echo "the API is connected as '$API_USERS', expected only sagat_app" >&2; exit 1; }
echo "ok: the API is connected only as sagat_app"

runtime_sql() {  # runtime_sql <statement>: run as the runtime role
  "${COMPOSE[@]}" exec -T -e PGPASSWORD="$APP_PASSWORD" postgres \
    psql -h 127.0.0.1 -U sagat_app -d "$DB_NAME" -v ON_ERROR_STOP=1 -tAc "$1"
}
runtime_sql "SELECT count(*) FROM audit_records" >/dev/null
for statement in "DELETE FROM audit_records" "TRUNCATE audit_records" "DROP TABLE audit_records" \
                 "ALTER TABLE audit_records DISABLE TRIGGER ALL"; do
  if runtime_sql "$statement" >/dev/null 2>&1; then
    echo "the runtime role was able to run: $statement" >&2
    exit 1
  fi
done
echo "ok: the runtime role can read the audit trail but cannot delete, truncate, drop or unprotect it"

echo "--- the API container's own health probe agrees"
control_plane_healthy() {
  [ "$("${COMPOSE[@]}" ps control-plane --format '{{.Health}}')" = "healthy" ]
}
# The image's HEALTHCHECK has a 10 s start period, so give it time instead of sampling once.
wait_for "control-plane container reports healthy" control_plane_healthy

echo "--- the execution engine is healthy and listens for workers"
engine_healthy() {
  [ "$("${COMPOSE[@]}" ps execution-engine --format '{{.Health}}')" = "healthy" ]
}
wait_for "execution-engine container reports healthy" engine_healthy
GRPC_PORT=$(env_value SAGAT_GRPC_PORT 19000)
wait_for "worker gRPC port ($GRPC_PORT) accepts connections" bash -c "exec 3<>/dev/tcp/127.0.0.1/$GRPC_PORT"
# Both processes connect as the runtime role (checked above for every connection of the database),
# and a worker call without credentials is refused by the gRPC server, not by a proxy: the HTTP/2
# preface is answered with a gRPC status in the response trailers.
ENGINE_USERS=$("${COMPOSE[@]}" exec -T postgres psql -U "$DB_OWNER" -d "$DB_NAME" -tAc \
  "SELECT string_agg(DISTINCT usename, ',') FROM pg_stat_activity WHERE datname = '$DB_NAME' AND usename <> '$DB_OWNER'")
[ "$ENGINE_USERS" = "sagat_app" ] || { echo "unexpected database users: '$ENGINE_USERS'" >&2; exit 1; }
echo "ok: every non-owner connection (API and engine) is the runtime role"

echo "--- the development identity provider signs a token the API accepts, and the seed left a namespace"
IDP="http://127.0.0.1:$(env_value SAGAT_IDP_PORT 18090)"
STUDIO="http://127.0.0.1:$(env_value SAGAT_API_PORT 18080)/studio/"
wait_for "development identity provider serves its keys ($IDP/.well-known/jwks.json)" \
  curl -fsS "$IDP/.well-known/jwks.json"
# The token is held in a variable and never printed, and never in the arguments of a process (they
# are visible to every user of the machine): curl reads its header from a file descriptor.
TOKEN=$(curl -fsS -X POST "$IDP/token" -H 'content-type: application/json' \
  -d '{"subject":"dev-admin","tenant":"dev"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')
authed_curl() {  # authed_curl <curl arguments...>: with the token of dev-admin
  curl -fsS --config <(printf 'header = "Authorization: Bearer %s"\n' "$TOKEN") "$@"
}
authed_curl "$API/auth/context" \
  | python3 -c 'import json,sys; c=json.load(sys.stdin); assert (c["tenant"], c["role"]) == ("dev", "admin"), c'
echo "ok: the API knows dev-admin as the administrator of tenant dev"
authed_curl "$API/namespaces" \
  | python3 -c 'import json,sys; s=[n["slug"] for n in json.load(sys.stdin)["items"]]; assert "payments" in s, s'
echo "ok: the namespace payments is there (the seed ran twice and made it once)"
BAD=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer not-a-token" "$API/auth/context")
[ "$BAD" = "401" ] || { echo "a token nobody signed was answered $BAD, expected 401" >&2; exit 1; }
echo "ok: a token nobody signed is refused"
curl -fsS "$STUDIO" | grep -q '<div id="root">' \
  || { echo "the Studio is not served at $STUDIO" >&2; exit 1; }
echo "ok: the Studio is served at $STUDIO"
IDP_HOST=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: evil.example' "$IDP/.well-known/jwks.json")
[ "$IDP_HOST" = "403" ] || { echo "the provider answered a stranger's Host header with $IDP_HOST, expected 403" >&2; exit 1; }
echo "ok: the development identity provider refuses a Host header it is not known by"

if [ -n "${SAGAT_BIN:-}" ]; then
  echo "--- the first day with the CLI: doctor, publish, serve, run"
  SAGAT_BIN=$(cd "$(dirname "$SAGAT_BIN")" && pwd)/$(basename "$SAGAT_BIN")
  export SAGAT_SERVER="$API" SAGAT_GRPC="http://127.0.0.1:$(env_value SAGAT_GRPC_PORT 19000)" SAGAT_NAMESPACE=payments
  export SAGAT_TOKEN="$TOKEN"
  WORK=$(mktemp -d)
  ( cd "$WORK"
    "$SAGAT_BIN" doctor --nats "nats://127.0.0.1:$(env_value SAGAT_NATS_PORT 14222)"
    "$SAGAT_BIN" new hello-world
    "$SAGAT_BIN" workflow publish hello-world/workflow.json
    "$SAGAT_BIN" worker dev --mocks hello-world/mocks.yaml --workflow hello-world/workflow.json \
      --bind --max-tasks 2 --idle-exit 60 > worker.out 2>&1 &
    WORKER=$!
    ran=0; "$SAGAT_BIN" workflow run hello-world --input '{"name":"Ada"}' --wait --timeout 60 | tee run.out || ran=$?
    served=0; wait "$WORKER" || served=$?
    cat worker.out
    [ "$ran" = 0 ] && [ "$served" = 0 ] || { echo "the run ended $ran, the worker $served" >&2; exit 1; }
    grep -q "SUCCEEDED" run.out
    grep -q "served 2 tasks" worker.out )
  rm -rf "$WORK"
  echo "ok: a two-step workflow was published, served and run to the end"
else
  echo "skipped: the first day with the CLI (set SAGAT_BIN to a built sagat to run it)"
fi

echo "--- smoke test passed"
