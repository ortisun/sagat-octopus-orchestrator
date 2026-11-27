//! Design 041, task T041-6: the credential shapes of the script logs are reachable from outside the domain, so that the
//! step-log ingest scrubs with the same function instead of a second list of shapes.

use sagat_domain::scripts::redact_credentials;

#[test]
fn the_script_logs_credential_shapes_are_public_and_hide_each_shape() {
    let aws = redact_credentials("key AKIAIOSFODNN7EXAMPLE end");
    assert!(!aws.contains("AKIAIOSFODNN7EXAMPLE"), "{aws}");
    let pem = redact_credentials(
        "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBg\n-----END PRIVATE KEY-----",
    );
    assert!(!pem.contains("MIIEvQIBADANBg"), "{pem}");
    let bearer = redact_credentials("Authorization: Bearer abcdef0123456789abcdef");
    assert!(!bearer.contains("abcdef0123456789abcdef"), "{bearer}");
    assert_eq!(redact_credentials("nothing secret"), "nothing secret");
}

/// Design 041, the security review of task T041-6: an armored PGP private key (`-----BEGIN PGP PRIVATE KEY BLOCK-----`)
/// is a private key block too.
#[test]
fn an_armored_pgp_private_key_is_hidden() {
    let pgp = redact_credentials(
        "x -----BEGIN PGP PRIVATE KEY BLOCK-----\nlQOYBGsecretbody\n-----END PGP PRIVATE KEY BLOCK----- y",
    );
    assert!(!pgp.contains("lQOYBGsecretbody"), "{pgp}");
    assert!(pgp.starts_with("x ") && pgp.ends_with(" y"), "{pgp}");
}
