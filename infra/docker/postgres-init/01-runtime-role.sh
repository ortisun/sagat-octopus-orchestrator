#!/bin/sh
# Runs once, when the PostgreSQL data volume is first initialised.
#
# Creates the least-privilege role the control plane serves requests with. The migrations grant it
# exactly what it needs (read tenants and memberships; create and read workspaces, namespaces and
# audit records). It owns nothing, so it cannot alter the schema or disable the audit triggers.
# The owner role ($POSTGRES_USER) is used only by the `migrate` and `seed` jobs.
set -eu

: "${SAGAT_APP_PASSWORD:?SAGAT_APP_PASSWORD must be set}"

# The password travels as a psql variable, quoted by psql itself (:'app_password'), never spliced
# into the SQL text.
psql -v ON_ERROR_STOP=1 -v app_password="$SAGAT_APP_PASSWORD" \
     --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<'SQL'
CREATE ROLE sagat_app LOGIN PASSWORD :'app_password'
    NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
SQL
