#!/usr/bin/env bash
# Smoke test of the local environment (US03): builds the image, starts the stack, checks that the
# API, PostgreSQL and NATS are healthy, that the seed is idempotent, then tears everything down.
#
#   scripts/smoke-compose.sh                 # uses infra/docker/.env.example
#   ENV_FILE=infra/docker/.env scripts/smoke-compose.sh
set -euo pipefail

cd "$(dirname "$0")/.."
ENV_FILE="${ENV_FILE:-infra/docker/.env.example}"
COMPOSE=(docker compose -f infra/docker/compose.yaml --env-file "$ENV_FILE")

# Read the published host ports the same way compose resolves them.
set -a; . "$ENV_FILE"; set +a
API="http://127.0.0.1:${SAGAT_API_PORT:-18080}/v1"
NATS_MONITOR="http://127.0.0.1:${SAGAT_NATS_MONITOR_PORT:-18222}"

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

echo "--- building and starting the stack"
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
TENANTS=$("${COMPOSE[@]}" exec -T postgres psql -U "${POSTGRES_USER:-sagat}" -d "${POSTGRES_DB:-sagat}" -tAc "SELECT count(*) FROM tenants")
[ "$TENANTS" = "1" ] || { echo "expected exactly 1 seeded tenant, found $TENANTS" >&2; exit 1; }
echo "ok: exactly one tenant after seeding twice"

echo "--- the API container's own health probe agrees"
control_plane_healthy() {
  [ "$("${COMPOSE[@]}" ps control-plane --format '{{.Health}}')" = "healthy" ]
}
# The image's HEALTHCHECK has a 10 s start period, so give it time instead of sampling once.
wait_for "control-plane container reports healthy" control_plane_healthy

echo "--- smoke test passed"
