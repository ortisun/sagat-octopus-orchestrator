//! The owner's side of the role of the automatic handover (feature 018, `T018-084`, ADR-0091 decision 1): what `migrate` runs after
//! the migrations, as the schema-owning role. It is a module of its own so that the privilege audit of `sagat_handover` reads only
//! what the job runs (architecture review Major 2): ensure the role, grant it again (refusing a role wider than the design), set the
//! owner's floor of the live engines, and set the role's login from a SCRAM verifier the caller computed, only while the handover is
//! `LEGACY`, the role was never retired and it is narrow (D2264, D2269, D2272, D2275).

use sqlx::{ConnectOptions as _, Connection as _, PgPool};

/// What `migrate` found of the role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleEnsured {
    Exists,
    Created,
    /// Missing, and the migrating role may not create roles: the operator must (G2249).
    NotPermitted,
}

/// What the grant function did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleGrants {
    Granted,
    /// The role is wider than the design (an attribute beyond LOGIN, a membership, a member that can act as it, an object it owns):
    /// it was given nothing (D2275).
    Wider,
    Missing,
}

fn unknown(what: &str, outcome: &str) -> sqlx::Error {
    sqlx::Error::Protocol(format!(
        "the database gave {what} an outcome this build does not know: {outcome:.32}"
    ))
}

/// Ensures `sagat_handover` and grants it everything it needs, every run (D2264).
///
/// # Errors
/// The driver's; an outcome this build does not know is an error, never a default.
pub async fn ensure_handover_role(pool: &PgPool) -> Result<(RoleEnsured, RoleGrants), sqlx::Error> {
    let ensured: String = sqlx::query_scalar("SELECT schedule_trigger_handover_role_ensure()")
        .fetch_one(pool)
        .await?;
    let granted: String = sqlx::query_scalar("SELECT schedule_trigger_handover_role_grants()")
        .fetch_one(pool)
        .await?;
    let ensured = match ensured.as_str() {
        "CREATED" => RoleEnsured::Created,
        "NOT_PERMITTED" => RoleEnsured::NotPermitted,
        "EXISTS" => RoleEnsured::Exists,
        other => return Err(unknown("the ensure of the role", other)),
    };
    let granted = match granted.as_str() {
        "GRANTED" => RoleGrants::Granted,
        "WIDER" => RoleGrants::Wider,
        "MISSING" => RoleGrants::Missing,
        other => return Err(unknown("the grant of the role", other)),
    };
    Ok((ensured, granted))
}

/// Sets the owner's floor of the live engines the automatic handover counts with (`SAGAT_HANDOVER_MIN_LIVE`, security review m3):
/// a caller of the round function cannot ask for fewer.
///
/// # Errors
/// The driver's; a floor outside 1 to 1000 is refused by the database (`22023`).
pub async fn set_handover_floor(pool: &PgPool, min_live: u32) -> Result<u32, sqlx::Error> {
    let floor: i32 = sqlx::query_scalar("SELECT schedule_trigger_handover_set_floor($1)")
        .bind(i32::try_from(min_live).unwrap_or(i32::MAX))
        .fetch_one(pool)
        .await?;
    Ok(u32::try_from(floor).unwrap_or(0))
}

/// What setting the login did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginSet {
    /// `LOGIN` and the verifier are set.
    Set,
    /// The role was retired: left as it is (D2269).
    Retired,
    /// The cron triggers were handed over: left as it is (D2269).
    HandedOver,
    /// The role does not exist.
    NoRole,
    /// The role is wider than the design: left as it is, never logged in (security review M1).
    Wider,
    /// The migrating role may not alter it (`42501`, PostgreSQL 16 without `ADMIN OPTION`, D2274).
    NotPermitted,
}

/// Whether `text` is a SCRAM-SHA-256 verifier as PostgreSQL stores one: `SCRAM-SHA-256$<iterations>:<salt>$<StoredKey>:<ServerKey>`,
/// with base64 parts. Nothing else may reach the statement that sets the password, which cannot take a bind parameter.
#[must_use]
pub fn is_scram_verifier(text: &str) -> bool {
    let b64 = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    };
    let Some(rest) = text.strip_prefix("SCRAM-SHA-256$") else {
        return false;
    };
    let Some((head, keys)) = rest.split_once('$') else {
        return false;
    };
    let Some((iterations, salt)) = head.split_once(':') else {
        return false;
    };
    let Some((stored, server)) = keys.split_once(':') else {
        return false;
    };
    !iterations.is_empty()
        && iterations.bytes().all(|b| b.is_ascii_digit())
        && b64(salt)
        && b64(stored)
        && b64(server)
        && text.len() <= 512
}

/// Sets `LOGIN` and the password of `sagat_handover` from a SCRAM verifier, only while the handover is `LEGACY`, the role was never
/// retired and it is as narrow as decided, in one transaction that reads all three (D2269, security review M1). The clear password
/// never reaches the server (D2272), and the statement that carries the verifier runs on a connection of its own whose statement
/// logging is off, so the client logs it at no level either (security review m2).
///
/// # Errors
/// The driver's; [`sqlx::Error::Protocol`] for a text that is not a verifier.
pub async fn set_handover_login(pool: &PgPool, verifier: &str) -> Result<LoginSet, sqlx::Error> {
    if !is_scram_verifier(verifier) {
        return Err(sqlx::Error::Protocol(
            "the password of sagat_handover is set only from a SCRAM-SHA-256 verifier".into(),
        ));
    }
    let mut conn = pool
        .connect_options()
        .as_ref()
        .clone()
        .disable_statement_logging()
        .connect()
        .await?;
    let mut tx = conn.begin().await?;
    let state: String = sqlx::query_scalar("SELECT state FROM schedule_trigger_handover FOR SHARE")
        .fetch_one(&mut *tx)
        .await?;
    let retired: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM schedule_trigger_handover_retirement)")
            .fetch_one(&mut *tx)
            .await?;
    let narrow: String = sqlx::query_scalar("SELECT schedule_trigger_handover_role_narrow()")
        .fetch_one(&mut *tx)
        .await?;
    let answer = if retired {
        LoginSet::Retired
    } else if state != "LEGACY" {
        LoginSet::HandedOver
    } else {
        match narrow.as_str() {
            "MISSING" => LoginSet::NoRole,
            "WIDER" => LoginSet::Wider,
            "NARROW" => {
                // the verifier is checked above to hold only the characters of base64, digits, `:` and `$`: it cannot leave the literal
                let statement = format!("ALTER ROLE sagat_handover LOGIN PASSWORD '{verifier}'");
                match sqlx::query(sqlx::AssertSqlSafe(statement))
                    .execute(&mut *tx)
                    .await
                {
                    Ok(_) => LoginSet::Set,
                    Err(sqlx::Error::Database(db)) if db.code().as_deref() == Some("42501") => {
                        tx.rollback().await?;
                        conn.close().await?;
                        return Ok(LoginSet::NotPermitted);
                    }
                    Err(error) => return Err(error),
                }
            }
            other => return Err(unknown("the narrowness of the role", other)),
        }
    };
    tx.commit().await?;
    conn.close().await?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_scram_verifier_reaches_the_statement() {
        assert!(is_scram_verifier(
            "SCRAM-SHA-256$4096:c2FsdA==$c3RvcmVk/+=:c2VydmVy"
        ));
        for hostile in [
            "",
            "md5abc",
            "SCRAM-SHA-256$4096:salt$stored",
            "SCRAM-SHA-256$40x6:c2FsdA==$a:b",
            "SCRAM-SHA-256$4096:c2FsdA==$a:b'; DROP ROLE x; --",
            "SCRAM-SHA-256$4096:c2Fs dA==$a:b",
            "plain password",
        ] {
            assert!(!is_scram_verifier(hostile), "{hostile:?}");
        }
    }
}
