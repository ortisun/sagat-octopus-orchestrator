-- FR-004-004: each inbound webhook endpoint selects exactly one persisted authentication profile.
-- Existing rows stay HMAC. Secrets remain references by name; OIDC verifier configuration and TLS
-- trust roots are deployment configuration and never live in this table.

ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_profile_valid;
ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_secret_format;

ALTER TABLE event_endpoints ALTER COLUMN secret_name DROP NOT NULL;
ALTER TABLE event_endpoints ADD COLUMN oidc_profile text;
ALTER TABLE event_endpoints ADD COLUMN oidc_subject text;
ALTER TABLE event_endpoints ADD COLUMN mtls_certificate_sha256 text;

ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_profile_valid
    CHECK (profile IN ('HMAC', 'OIDC', 'MTLS'));
ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_authentication_consistent CHECK (
    (
        profile = 'HMAC'
        AND secret_name IS NOT NULL
        AND oidc_profile IS NULL
        AND oidc_subject IS NULL
        AND mtls_certificate_sha256 IS NULL
    ) OR (
        profile = 'OIDC'
        AND secret_name IS NULL
        AND oidc_profile IS NOT NULL
        AND oidc_subject IS NOT NULL
        AND mtls_certificate_sha256 IS NULL
    ) OR (
        profile = 'MTLS'
        AND secret_name IS NULL
        AND oidc_profile IS NULL
        AND oidc_subject IS NULL
        AND mtls_certificate_sha256 IS NOT NULL
    )
);
ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_secret_format CHECK (
    secret_name IS NULL OR secret_name ~ '^[a-z][a-z0-9._-]{0,63}$'
);
ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_oidc_profile_format CHECK (
    oidc_profile IS NULL OR (
        oidc_profile ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$'
        AND oidc_profile !~ '--'
    )
);
ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_oidc_subject_format CHECK (
    oidc_subject IS NULL OR (
        octet_length(oidc_subject) BETWEEN 1 AND 255
        AND oidc_subject !~ '[[:cntrl:]]'
    )
);
ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_mtls_fingerprint_format CHECK (
    mtls_certificate_sha256 IS NULL OR mtls_certificate_sha256 ~ '^[0-9a-f]{64}$'
);
