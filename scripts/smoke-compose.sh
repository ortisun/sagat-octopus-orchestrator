#!/usr/bin/env bash
# Smoke test of the local environment (US03): builds the image, starts the stack, checks that the
# API, PostgreSQL and NATS are healthy, that migrations and the seed are idempotent and that the API
# runs as the least-privilege database role, then tears everything down.
#
#   scripts/smoke-compose.sh                 # uses infra/docker/.env.example
#   ENV_FILE=infra/docker/.env scripts/smoke-compose.sh
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

echo "--- smoke test passed"
