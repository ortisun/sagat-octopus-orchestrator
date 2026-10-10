-- Feature 005, inline scripts: the scripts authors edit, the immutable versions they publish, the
-- signed artifacts the builds make, the jobs that build and test them in the sandbox, and the
-- record of what ran (FR-005-001..008). Every foreign key is tenant-scoped, so the database itself
-- refuses a version of another tenant's script.

-- The identity of a sandbox runner joins the built-in roles.
ALTER TABLE memberships DROP CONSTRAINT memberships_role_valid;
ALTER TABLE memberships
    ADD CONSTRAINT memberships_role_valid
    CHECK (role IN ('admin', 'developer', 'operator', 'viewer', 'worker', 'runner'));

-- A script and its draft: the one thing here that changes. A change is a compare-and-swap on the
-- revision, so two editors never silently overwrite each other.
CREATE TABLE scripts (
    id             uuid        PRIMARY KEY,
    tenant_id      uuid        NOT NULL REFERENCES tenants (id),
    namespace_id   uuid        NOT NULL,
    name           text        NOT NULL,
    language       text        NOT NULL,
    source         text        NOT NULL,
    lockfile       text,
    input_schema   bytea,
    output_schema  bytea,
    -- What the code is held to, as the platform writes it (`v1;memory=..;..`).
    policy         text        NOT NULL,
    retention_days smallint    NOT NULL,
    revision       bigint      NOT NULL DEFAULT 1,
    latest_version integer,
    created_by     text        NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at     timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT scripts_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT scripts_name_uniq UNIQUE (tenant_id, namespace_id, name),
    CONSTRAINT scripts_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT scripts_language_valid CHECK (language IN ('typescript', 'python', 'csharp', 'go', 'java')),
    CONSTRAINT scripts_name_format CHECK (name ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$'),
    CONSTRAINT scripts_source_size CHECK (octet_length(source) BETWEEN 1 AND 262144),
    CONSTRAINT scripts_lockfile_size CHECK (lockfile IS NULL OR octet_length(lockfile) <= 262144),
    CONSTRAINT scripts_schema_size CHECK (
        (input_schema IS NULL OR octet_length(input_schema) <= 65536)
        AND (output_schema IS NULL OR octet_length(output_schema) <= 65536)),
    CONSTRAINT scripts_retention_range CHECK (retention_days BETWEEN 1 AND 365),
    CONSTRAINT scripts_revision_positive CHECK (revision >= 1)
);

-- What a build made, once per digest and tenant. The bytes are what the runner runs, the signature
-- is what makes it trusted: a row written by anything but the control plane does not verify.
CREATE TABLE script_artifacts (
    tenant_id  uuid        NOT NULL REFERENCES tenants (id),
    digest     text        NOT NULL,
    language   text        NOT NULL,
    content    bytea       NOT NULL,
    signature  bytea       NOT NULL,
    key_id     text        NOT NULL,
    -- CycloneDX: the toolchain and every package the lockfile pins.
    sbom       bytea       NOT NULL,
    toolchain  text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, digest),
    CONSTRAINT script_artifacts_digest_format CHECK (digest ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT script_artifacts_size CHECK (octet_length(content) BETWEEN 1 AND 2097152),
    CONSTRAINT script_artifacts_signature_length CHECK (octet_length(signature) = 64),
    CONSTRAINT script_artifacts_sbom_size CHECK (octet_length(sbom) <= 262144)
);

-- A published version: the draft as it was, frozen. The only change it ever sees is the one that
-- records how its build ended (BUILDING to READY or FAILED); the trigger below refuses all others.
CREATE TABLE script_versions (
    id             uuid        PRIMARY KEY,
    tenant_id      uuid        NOT NULL,
    namespace_id   uuid        NOT NULL,
    script_id      uuid        NOT NULL,
    number         integer     NOT NULL,
    language       text        NOT NULL,
    status         text        NOT NULL DEFAULT 'BUILDING',
    source         text        NOT NULL,
    lockfile       text,
    input_schema   bytea,
    output_schema  bytea,
    policy         text        NOT NULL,
    retention_days smallint    NOT NULL,
    -- Of everything above: two publications with the same one are one version.
    content_digest text        NOT NULL,
    build_key      text,
    artifact_digest text,
    key_id         text,
    toolchain      text,
    build_log      text,
    failure        text,
    created_by     text        NOT NULL,
    created_at     timestamptz NOT NULL,
    built_at       timestamptz,
    CONSTRAINT script_versions_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT script_versions_number_uniq UNIQUE (tenant_id, script_id, number),
    CONSTRAINT script_versions_script_fk FOREIGN KEY (tenant_id, script_id)
        REFERENCES scripts (tenant_id, id),
    CONSTRAINT script_versions_artifact_fk FOREIGN KEY (tenant_id, artifact_digest)
        REFERENCES script_artifacts (tenant_id, digest),
    CONSTRAINT script_versions_status_valid CHECK (status IN ('BUILDING', 'READY', 'FAILED')),
    CONSTRAINT script_versions_number_positive CHECK (number >= 1),
    CONSTRAINT script_versions_ready_has_artifact CHECK ((status = 'READY') = (artifact_digest IS NOT NULL)),
    CONSTRAINT script_versions_failed_has_reason CHECK ((status = 'FAILED') = (failure IS NOT NULL)),
    CONSTRAINT script_versions_digests_format CHECK (
        content_digest ~ '^sha256:[0-9a-f]{64}$'
        AND (build_key IS NULL OR build_key ~ '^sha256:[0-9a-f]{64}$')
        AND (artifact_digest IS NULL OR artifact_digest ~ '^sha256:[0-9a-f]{64}$'))
);
-- The build cache: a ready version of the tenant built with this key.
CREATE INDEX script_versions_build_key_idx ON script_versions (tenant_id, build_key)
    WHERE status = 'READY';
-- Who may be told that an artifact exists, and how long its executions are kept.
CREATE INDEX script_versions_artifact_idx ON script_versions (tenant_id, artifact_digest)
    WHERE artifact_digest IS NOT NULL;

CREATE FUNCTION script_versions_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP <> 'UPDATE' THEN
        RAISE EXCEPTION 'script_versions is immutable (% attempted)', TG_OP
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.status <> 'BUILDING' THEN
        RAISE EXCEPTION 'a finished script version does not change'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF (NEW.id, NEW.tenant_id, NEW.namespace_id, NEW.script_id, NEW.number, NEW.language,
        NEW.source, NEW.lockfile, NEW.input_schema, NEW.output_schema, NEW.policy,
        NEW.retention_days, NEW.content_digest, NEW.created_by, NEW.created_at)
       IS DISTINCT FROM
       (OLD.id, OLD.tenant_id, OLD.namespace_id, OLD.script_id, OLD.number, OLD.language,
        OLD.source, OLD.lockfile, OLD.input_schema, OLD.output_schema, OLD.policy,
        OLD.retention_days, OLD.content_digest, OLD.created_by, OLD.created_at) THEN
        RAISE EXCEPTION 'a published script version keeps its source, lockfile, schemas and policy'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER script_versions_immutable
    BEFORE UPDATE OR DELETE ON script_versions
    FOR EACH ROW EXECUTE FUNCTION script_versions_guard();
CREATE TRIGGER script_versions_no_truncate
    BEFORE TRUNCATE ON script_versions
    FOR EACH STATEMENT EXECUTE FUNCTION script_versions_guard();
ALTER TABLE script_versions ENABLE ALWAYS TRIGGER script_versions_immutable;
ALTER TABLE script_versions ENABLE ALWAYS TRIGGER script_versions_no_truncate;

CREATE FUNCTION script_artifacts_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'script_artifacts is immutable (% attempted)', TG_OP
        USING ERRCODE = 'restrict_violation';
END;
$$;
CREATE TRIGGER script_artifacts_immutable
    BEFORE UPDATE OR DELETE ON script_artifacts
    FOR EACH ROW EXECUTE FUNCTION script_artifacts_reject_mutation();
CREATE TRIGGER script_artifacts_no_truncate
    BEFORE TRUNCATE ON script_artifacts
    FOR EACH STATEMENT EXECUTE FUNCTION script_artifacts_reject_mutation();
ALTER TABLE script_artifacts ENABLE ALWAYS TRIGGER script_artifacts_immutable;
ALTER TABLE script_artifacts ENABLE ALWAYS TRIGGER script_artifacts_no_truncate;

-- The build of a version. A runner holds it under a lease: the hash of a token only that runner
-- knows, and a time after which another runner may take it. Everything the holder reports is
-- checked against the token, so a runner whose lease ran out cannot finish what another has taken.
CREATE TABLE script_build_jobs (
    id               uuid        PRIMARY KEY,
    tenant_id        uuid        NOT NULL,
    namespace_id     uuid        NOT NULL,
    version_id       uuid        NOT NULL,
    language         text        NOT NULL,
    status           text        NOT NULL DEFAULT 'QUEUED',
    attempts         integer     NOT NULL DEFAULT 0,
    lease_token_hash bytea,
    lease_expires_at timestamptz,
    created_at       timestamptz NOT NULL,
    finished_at      timestamptz,
    CONSTRAINT script_build_jobs_version_uniq UNIQUE (tenant_id, version_id),
    CONSTRAINT script_build_jobs_version_fk FOREIGN KEY (tenant_id, version_id)
        REFERENCES script_versions (tenant_id, id),
    CONSTRAINT script_build_jobs_status_valid CHECK (status IN ('QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED')),
    CONSTRAINT script_build_jobs_finished_consistent CHECK (
        (finished_at IS NOT NULL) = (status IN ('SUCCEEDED', 'FAILED')))
);
CREATE INDEX script_build_jobs_claim_idx ON script_build_jobs (tenant_id, created_at)
    WHERE status IN ('QUEUED', 'RUNNING');
CREATE INDEX script_build_jobs_namespace_idx ON script_build_jobs (tenant_id, namespace_id)
    WHERE status IN ('QUEUED', 'RUNNING');

-- A trial of a draft without publishing it: the draft as it was, an input, and what came of it.
CREATE TABLE script_test_runs (
    id                    uuid        PRIMARY KEY,
    tenant_id             uuid        NOT NULL,
    namespace_id          uuid        NOT NULL,
    script_id             uuid        NOT NULL,
    language              text        NOT NULL,
    status                text        NOT NULL DEFAULT 'QUEUED',
    source                text        NOT NULL,
    lockfile              text,
    input_schema          bytea,
    output_schema         bytea,
    policy                text        NOT NULL,
    retention_days        smallint    NOT NULL,
    input                 bytea       NOT NULL,
    result                bytea,
    failure               text,
    log                   text        NOT NULL DEFAULT '',
    log_truncated         boolean     NOT NULL DEFAULT false,
    denied                text[]      NOT NULL DEFAULT '{}',
    exit_code             bigint,
    wall_ms               bigint,
    start_ms              bigint,
    memory_peak_bytes     bigint,
    inferred_input_schema  bytea,
    inferred_output_schema bytea,
    attempts              integer     NOT NULL DEFAULT 0,
    lease_token_hash      bytea,
    lease_expires_at      timestamptz,
    created_by            text        NOT NULL,
    created_at            timestamptz NOT NULL,
    -- A run no runner took by then is failed as unavailable.
    deadline_at           timestamptz NOT NULL,
    finished_at           timestamptz,
    CONSTRAINT script_test_runs_script_fk FOREIGN KEY (tenant_id, script_id)
        REFERENCES scripts (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT script_test_runs_status_valid CHECK (status IN ('QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED')),
    CONSTRAINT script_test_runs_finished_consistent CHECK (
        (finished_at IS NOT NULL) = (status IN ('SUCCEEDED', 'FAILED'))),
    CONSTRAINT script_test_runs_input_size CHECK (octet_length(input) <= 245760),
    CONSTRAINT script_test_runs_result_size CHECK (result IS NULL OR octet_length(result) <= 245760),
    CONSTRAINT script_test_runs_log_size CHECK (octet_length(log) <= 1048576)
);
CREATE INDEX script_test_runs_claim_idx ON script_test_runs (tenant_id, created_at)
    WHERE status IN ('QUEUED', 'RUNNING');
CREATE INDEX script_test_runs_namespace_idx ON script_test_runs (tenant_id, namespace_id)
    WHERE status IN ('QUEUED', 'RUNNING');
CREATE INDEX script_test_runs_listing_idx ON script_test_runs (tenant_id, script_id, id DESC);
CREATE INDEX script_test_runs_purge_idx ON script_test_runs (finished_at) WHERE finished_at IS NOT NULL;

-- What a published script did, once per run, node and attempt, kept until `expires_at`. The log is
-- redacted and bounded; the result is the step's own output and is not copied here.
CREATE TABLE script_executions (
    id                uuid        PRIMARY KEY,
    tenant_id         uuid        NOT NULL,
    namespace_id      uuid        NOT NULL,
    digest            text        NOT NULL,
    run_id            uuid,
    node              text,
    attempt           integer,
    status            text        NOT NULL,
    failure           text,
    exit_code         bigint,
    log               text        NOT NULL DEFAULT '',
    log_truncated     boolean     NOT NULL DEFAULT false,
    denied            text[]      NOT NULL DEFAULT '{}',
    wall_ms           bigint      NOT NULL,
    start_ms          bigint      NOT NULL,
    memory_peak_bytes bigint,
    trace_id          text,
    created_at        timestamptz NOT NULL,
    expires_at        timestamptz NOT NULL,
    CONSTRAINT script_executions_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT script_executions_artifact_fk FOREIGN KEY (tenant_id, digest)
        REFERENCES script_artifacts (tenant_id, digest),
    CONSTRAINT script_executions_status_valid CHECK (status IN ('SUCCEEDED', 'FAILED')),
    CONSTRAINT script_executions_log_size CHECK (octet_length(log) <= 1048576)
);
CREATE UNIQUE INDEX script_executions_step_uniq ON script_executions (tenant_id, run_id, node, attempt)
    WHERE run_id IS NOT NULL;
CREATE INDEX script_executions_namespace_idx ON script_executions (tenant_id, namespace_id, id DESC);
CREATE INDEX script_executions_digest_idx ON script_executions (tenant_id, digest, id DESC);
CREATE INDEX script_executions_expiry_idx ON script_executions (expires_at);

-- Scripts and the builds and test runs in progress are quota resources, counted per namespace.
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors', 'scripts', 'script_jobs'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints',
                                         'webhook_subscriptions', 'event_connectors', 'scripts',
                                         'script_jobs'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors', 'scripts', 'script_jobs'));

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON scripts, script_build_jobs, script_test_runs TO sagat_app;
        GRANT SELECT, INSERT, UPDATE ON script_versions TO sagat_app;
        GRANT SELECT, INSERT ON script_artifacts TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON script_executions TO sagat_app;
    END IF;
END
$grants$;
