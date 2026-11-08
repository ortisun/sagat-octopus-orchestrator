//! The answer to `ResolveDatabase` for a lake step, read into the connection the driver takes (feature 035, T035-058, D035-143).
//!
//! The SQL profile has its own reader (`capability::decode`), which wants a PostgreSQL answer. A lake answer is checked here: it is
//! for this runner's namespace, it names a connection and a revision, it carries nothing of SQL, and its Iceberg connection is one
//! the runner can open. **Every secret of the answer is zeroised on every path**, the ones that were read and the ones that were
//! refused.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the lake attempt calls it, and the serve loop calls that (G5)"
    )
)]

use sagat_api_grpc::pb;
use sagat_domain::{database::DatabaseFailure, identity::NamespaceId};
use zeroize::Zeroize as _;

use super::connection::{ConvertError, Converted, convert};

/// The connection of a lake attempt, or the failure to report for it. The answer is consumed and its secrets are gone.
///
/// # Errors
/// `DATABASE_CAPABILITY_UNAVAILABLE` for an answer that is not a lake answer for this runner's namespace, or a connection of a kind
/// the runner cannot open yet; `DATABASE_CONFIGURATION_INVALID` for a connection that is not complete; and
/// `DATABASE_CREDENTIAL_UNAVAILABLE` for a secret that is not a document the runner reads.
pub(crate) fn decode(
    mut response: pb::ResolvedDatabase,
    namespace: NamespaceId,
) -> Result<Converted, DatabaseFailure> {
    decode_in_place(&mut response, namespace)
}

/// As [`decode`], on an answer the caller keeps (to look at what is left of it).
pub(crate) fn decode_in_place(
    response: &mut pb::ResolvedDatabase,
    namespace: NamespaceId,
) -> Result<Converted, DatabaseFailure> {
    let converted = read(response, namespace);
    scrub(response);
    converted
}

fn read(
    response: &mut pb::ResolvedDatabase,
    namespace: NamespaceId,
) -> Result<Converted, DatabaseFailure> {
    if response.namespace_id.parse::<NamespaceId>().ok() != Some(namespace)
        || response.connection_id.parse::<uuid::Uuid>().is_err()
        || response.connection_revision == 0
    {
        return Err(DatabaseFailure::Capability);
    }
    // a lake answer carries nothing of SQL, and nothing of the other lake kinds: an answer of another shape is not for this profile
    if response.postgres.is_some()
        || response.credential.is_some()
        || response.object_storage.is_some()
        || response.clickhouse.is_some()
    {
        return Err(DatabaseFailure::Capability);
    }
    let iceberg = response
        .iceberg
        .as_mut()
        .ok_or(DatabaseFailure::Capability)?;
    convert(iceberg).map_err(|error| match error {
        ConvertError::Shape => DatabaseFailure::Configuration,
        ConvertError::Unsupported => DatabaseFailure::Capability,
        ConvertError::Secret => DatabaseFailure::Credential,
    })
}

/// Zeroises, in place, every secret an answer carries: the lake maps, and the SQL credential an answer of the wrong shape may carry.
fn scrub(response: &mut pb::ResolvedDatabase) {
    if let Some(credential) = &mut response.credential {
        credential.value.zeroize();
    }
    let lake = [
        response.iceberg.as_mut().map(|lake| &mut lake.credentials),
        response
            .object_storage
            .as_mut()
            .map(|lake| &mut lake.credentials),
        response
            .clickhouse
            .as_mut()
            .map(|lake| &mut lake.credentials),
    ];
    for credentials in lake.into_iter().flatten() {
        for bytes in credentials.values_mut() {
            bytes.zeroize();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::fixtures::{SECRET, resolved_database};
    use super::*;
    use pb::iceberg_connection::Catalog;

    const NAMESPACE: &str = "00000000-0000-0000-0000-000000000001";

    fn namespace() -> NamespaceId {
        NAMESPACE.parse().unwrap()
    }

    fn answer() -> pb::ResolvedDatabase {
        resolved_database(NAMESPACE)
    }

    /// Whether any byte of any secret the answer still carries is not zero.
    fn holds_a_secret(response: &pb::ResolvedDatabase) -> bool {
        let lake = [
            response.iceberg.as_ref().map(|lake| &lake.credentials),
            response
                .object_storage
                .as_ref()
                .map(|lake| &lake.credentials),
            response.clickhouse.as_ref().map(|lake| &lake.credentials),
        ];
        lake.into_iter()
            .flatten()
            .flat_map(std::collections::HashMap::values)
            .any(|bytes| bytes.iter().any(|byte| *byte != 0))
            || response
                .credential
                .as_ref()
                .is_some_and(|credential| credential.value.iter().any(|byte| *byte != 0))
    }

    fn refused(mut response: pb::ResolvedDatabase) -> DatabaseFailure {
        let result = decode_in_place(&mut response, namespace());
        assert!(
            !holds_a_secret(&response),
            "the secrets of a refused answer are gone too"
        );
        match result {
            Err(failure) => failure,
            Ok(_) => panic!("the answer was read"),
        }
    }

    #[test]
    fn a_lake_answer_for_this_namespace_is_read_and_its_secrets_are_gone() {
        let mut response = answer();
        let converted = decode_in_place(&mut response, namespace()).expect("read");
        assert_eq!(converted.warehouse_locations, ["s3://lake/warehouse/"]);
        assert!(
            !converted.catalog_secret.is_empty(),
            "the converted connection holds what it needs"
        );
        assert!(!holds_a_secret(&response), "the message holds none");
        assert!(decode(answer(), namespace()).is_ok());
    }

    #[test]
    fn an_answer_for_another_namespace_or_with_no_connection_is_not_read() {
        let mut other = answer();
        other.namespace_id = "00000000-0000-0000-0000-000000000002".into();
        assert_eq!(refused(other), DatabaseFailure::Capability);
        let mut unrevised = answer();
        unrevised.connection_revision = 0;
        assert_eq!(refused(unrevised), DatabaseFailure::Capability);
        let mut nameless = answer();
        nameless.connection_id = "not an id".into();
        assert_eq!(refused(nameless), DatabaseFailure::Capability);
        let mut empty = answer();
        empty.iceberg = None;
        assert_eq!(refused(empty), DatabaseFailure::Capability);
    }

    #[test]
    fn an_answer_that_carries_sql_or_another_lake_kind_is_not_for_this_profile() {
        let mut sql = answer();
        sql.postgres = Some(pb::PostgresConnection::default());
        assert_eq!(refused(sql), DatabaseFailure::Capability);
        let mut credential = answer();
        credential.credential = Some(pb::ResolvedSecret {
            name: "db-credentials".into(),
            version: 1,
            value: SECRET.as_bytes().to_vec(),
        });
        assert_eq!(
            refused(credential),
            DatabaseFailure::Capability,
            "and the SQL credential it carried is zeroised"
        );
        let mut other = answer();
        other.object_storage = Some(pb::ObjectStorageConnection::default());
        assert_eq!(refused(other), DatabaseFailure::Capability);
    }

    #[test]
    fn a_connection_the_runner_cannot_read_is_named_by_what_is_wrong_with_it() {
        let mut incomplete = answer();
        incomplete.iceberg.as_mut().unwrap().catalog = None;
        assert_eq!(refused(incomplete), DatabaseFailure::Configuration);
        let mut unsupported = answer();
        unsupported.iceberg.as_mut().unwrap().catalog =
            Some(Catalog::Glue(pb::IcebergGlueCatalog::default()));
        assert_eq!(refused(unsupported), DatabaseFailure::Capability);
        let mut secret = answer();
        secret
            .iceberg
            .as_mut()
            .unwrap()
            .credentials
            .insert("catalog".into(), b"not a document".to_vec());
        assert_eq!(refused(secret), DatabaseFailure::Credential);
    }
}
