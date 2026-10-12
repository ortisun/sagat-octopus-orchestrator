-- Older binaries only understand HMAC and a required signing secret. Refuse to erase OIDC/mTLS
-- configuration: operators must migrate or remove those endpoints explicitly before rollback.
DO $rollback$
BEGIN
    IF EXISTS (SELECT 1 FROM event_endpoints WHERE profile <> 'HMAC') THEN
        RAISE EXCEPTION
            'cannot roll back webhook authentication profiles while OIDC or mTLS endpoints exist';
    END IF;
END
$rollback$;

ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_mtls_fingerprint_format;
ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_oidc_subject_format;
ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_oidc_profile_format;
ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_authentication_consistent;
ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_secret_format;
ALTER TABLE event_endpoints DROP CONSTRAINT event_endpoints_profile_valid;

ALTER TABLE event_endpoints DROP COLUMN mtls_certificate_sha256;
ALTER TABLE event_endpoints DROP COLUMN oidc_subject;
ALTER TABLE event_endpoints DROP COLUMN oidc_profile;
ALTER TABLE event_endpoints ALTER COLUMN secret_name SET NOT NULL;

ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_profile_valid CHECK (profile IN ('HMAC'));
ALTER TABLE event_endpoints ADD CONSTRAINT event_endpoints_secret_format
    CHECK (secret_name ~ '^[a-z][a-z0-9._-]{0,63}$');
