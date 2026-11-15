//! The role of the automatic handover, as `migrate` handles it (feature 018, `T018-084`, ADR-0091 decision 1, D2264, D2269,
//! D2272, D2274).
//!
//! Every `migrate` run ensures `sagat_handover` and grants it again (in the adapter's `migrate`). Given `SAGAT_HANDOVER_PASSWORD`,
//! it then sets `LOGIN` and the password, **only while the handover is `LEGACY` and the role was never retired**, and it sends a
//! SCRAM-SHA-256 verifier it computed itself: the clear password never reaches the server, so neither `log_statement` nor an
//! error context can log it. Where PostgreSQL 16 refuses the alteration (`42501`), it warns with the remedy and goes on.

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use sagat_infra_postgres::{LoginSet, PgPool, set_handover_login};

use crate::RunError;
use crate::config::ConfigError;

/// The iterations of the verifier: PostgreSQL's default (`scram_iterations`).
pub const SCRAM_ITERATIONS: u32 = 4096;

const BLOCK: usize = 64;

/// HMAC-SHA-256 (RFC 2104) over the hashing the workspace already has.
#[must_use]
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0_u8; BLOCK];
    if key.len() > BLOCK {
        let digest: [u8; 32] = Sha256::digest(key).into();
        block[..32].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(message);
    let inner: [u8; 32] = inner.finalize().into();
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

/// PBKDF2-HMAC-SHA-256 (RFC 8018) of one block: the 32 bytes SCRAM-SHA-256 calls `SaltedPassword`.
#[must_use]
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut first = salt.to_vec();
    first.extend_from_slice(&1_u32.to_be_bytes());
    let mut u = hmac_sha256(password, &first);
    let mut out = u;
    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for (o, x) in out.iter_mut().zip(u) {
            *o ^= x;
        }
    }
    out
}

/// The verifier PostgreSQL stores for a SCRAM-SHA-256 password: `SCRAM-SHA-256$<iterations>:<salt>$<StoredKey>:<ServerKey>`
/// (RFC 5802, RFC 7677).
#[must_use]
pub fn scram_verifier(password: &str, salt: &[u8], iterations: u32) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    let salted = pbkdf2_sha256(password.as_bytes(), salt, iterations);
    let client_key = hmac_sha256(&salted, b"Client Key");
    let stored_key: [u8; 32] = Sha256::digest(client_key).into();
    let server_key = hmac_sha256(&salted, b"Server Key");
    format!(
        "SCRAM-SHA-256${iterations}:{}${}:{}",
        b64.encode(salt),
        b64.encode(stored_key),
        b64.encode(server_key)
    )
}

/// Why a password is not used: PostgreSQL normalises a password with SASLprep before it hashes it, and this verifier does not, so
/// only a password that SASLprep leaves as it is (printable ASCII) is accepted; one that is too short to resist an offline guess of
/// its verifier is refused too (G2254).
#[must_use]
pub fn unusable_password(password: &str) -> Option<&'static str> {
    if password.len() < 16 {
        return Some(
            "it is shorter than 16 characters (generate a long random one: the runbook says how)",
        );
    }
    if password.len() > 1024 {
        return Some("it is longer than 1024 characters");
    }
    if !password.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Some("it holds a character other than printable ASCII without spaces");
    }
    None
}

/// A random salt of 16 bytes, from the generator `uuid` uses for v4 identifiers (the operating system's).
fn salt() -> [u8; 16] {
    let mut salt = [0_u8; 16];
    salt.copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    // the version and variant bits of a v4 are fixed; a second identifier mixes them away
    for (s, x) in salt
        .iter_mut()
        .zip(uuid::Uuid::new_v4().as_bytes().iter().rev())
    {
        *s ^= x;
    }
    salt
}

/// What `migrate` did with `SAGAT_HANDOVER_PASSWORD`.
///
/// # Errors
/// [`RunError::Config`] for a password it will not use; [`RunError::Database`] for a failure of the database.
pub async fn set_login(pool: &PgPool, password: &str) -> Result<LoginSet, RunError> {
    if let Some(reason) = unusable_password(password) {
        return Err(ConfigError::Invalid {
            name: "SAGAT_HANDOVER_PASSWORD",
            reason: reason.to_owned(),
        }
        .into());
    }
    let verifier = scram_verifier(password, &salt(), SCRAM_ITERATIONS);
    let set = set_handover_login(pool, &verifier)
        .await
        .map_err(|error| RunError::Database(error.to_string()))?;
    match set {
        LoginSet::Set => tracing::info!(
            role = "sagat_handover",
            "the role of the automatic handover may log in (its password was set from a SCRAM verifier)"
        ),
        LoginSet::Retired => tracing::info!(
            role = "sagat_handover",
            "the role of the automatic handover was retired: SAGAT_HANDOVER_PASSWORD is ignored and the role stays without a login"
        ),
        LoginSet::HandedOver => tracing::info!(
            role = "sagat_handover",
            "the cron triggers were handed over: SAGAT_HANDOVER_PASSWORD is ignored; retire the role (docs/runbooks/schedules.md, 6.4)"
        ),
        LoginSet::NoRole => tracing::warn!(
            role = "sagat_handover",
            "SAGAT_HANDOVER_PASSWORD is set but the role does not exist and could not be created: create it once \
             (CREATE ROLE sagat_handover NOLOGIN) and run migrate again"
        ),
        LoginSet::NotPermitted => tracing::warn!(
            role = "sagat_handover",
            "the migrating role may not alter sagat_handover (PostgreSQL 16 requires ADMIN OPTION on a role another role created): \
             run GRANT sagat_handover TO <the migrating role> WITH ADMIN OPTION as the role's creator, or have its creator run \
             ALTER ROLE sagat_handover LOGIN PASSWORD '...'; migrate goes on"
        ),
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hmac_sha256_gives_the_rfc_4231_vectors() {
        // test case 2
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // test case 6: a key longer than the block
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn pbkdf2_sha256_gives_the_published_vectors() {
        assert_eq!(
            hex(&pbkdf2_sha256(b"password", b"salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            hex(&pbkdf2_sha256(b"password", b"salt", 4096)),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
    }

    #[test]
    fn the_verifier_has_the_form_postgresql_stores_and_never_holds_the_password() {
        let verifier = scram_verifier("a-long-password-with-@/#", &[7; 16], SCRAM_ITERATIONS);
        assert!(
            sagat_infra_postgres::is_scram_verifier(&verifier),
            "{verifier}"
        );
        assert!(verifier.starts_with("SCRAM-SHA-256$4096:BwcHBwcHBwcHBwcHBwcHBw==$"));
        assert!(!verifier.contains("a-long-password"));
        assert_ne!(
            verifier,
            scram_verifier("a-long-password-with-@/#", &salt(), SCRAM_ITERATIONS),
            "a random salt each time"
        );
    }

    #[test]
    fn a_password_the_verifier_cannot_stand_for_is_refused() {
        assert!(unusable_password("short").is_some());
        assert!(unusable_password("a password with spaces in it").is_some());
        assert!(unusable_password("pässwörd-pässwörd-pässwörd").is_some());
        assert!(unusable_password("Zx9@/#Zx9@/#Zx9@/#").is_none());
    }
}
