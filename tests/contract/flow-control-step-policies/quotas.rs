//! The quota resources everywhere they are listed agree with the code (T022-013, D022-040; architecture review m2 of
//! 2026-11-16): the canonical OpenAPI enums and the `CHECK` lists of the newest migration that restates them name exactly
//! `QuotaResource::ALL`. A resource added to one and not the others fails here, not on a replica that cannot read a row.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sagat_domain::quota::{QuotaResource, QuotaScope};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn all() -> BTreeSet<String> {
    QuotaResource::ALL
        .iter()
        .map(|r| r.as_str().to_owned())
        .collect()
}

fn names(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("an enum")
        .iter()
        .map(|v| v.as_str().expect("a name").to_owned())
        .collect()
}

#[test]
fn the_openapi_enums_name_every_resource_and_nothing_else() {
    let openapi: Value = serde_saphyr::from_str(
        &fs::read_to_string(root().join("specs/001-platform-foundation/contracts/openapi.yaml"))
            .expect("openapi"),
    )
    .expect("yaml");
    let parameter =
        names(&openapi["components"]["parameters"]["QuotaResourceName"]["schema"]["enum"]);
    let schema =
        names(&openapi["components"]["schemas"]["Quota"]["properties"]["resource"]["enum"]);
    assert_eq!(parameter, all(), "QuotaResourceName");
    assert_eq!(schema, all(), "Quota.resource");
}

/// The quoted names of the `CHECK (... IN (...))` of `constraint` in `sql`.
fn check_list(sql: &str, constraint: &str) -> BTreeSet<String> {
    let at = sql
        .rfind(&format!("ADD CONSTRAINT {constraint} CHECK"))
        .unwrap_or_else(|| panic!("{constraint} is restated"));
    let body = &sql[at..];
    // the statement ends at its `;`, after `)` or `) NOT VALID` (the CHECKs since design 043 are added NOT VALID)
    let body = &body[..body.find(';').expect("the end of the constraint")];
    body.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_newest_restatement_of_the_ceilings_tables_names_every_resource() {
    let directory = root().join("crates/infra-postgres/migrations");
    let mut ups: Vec<_> = fs::read_dir(&directory)
        .expect("migrations")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.to_string_lossy().ends_with(".up.sql"))
        .collect();
    ups.sort();
    let newest = ups
        .iter()
        .rev()
        .find(|path| {
            fs::read_to_string(path)
                .expect("sql")
                .contains("ADD CONSTRAINT quota_limits_resource_valid CHECK")
        })
        .expect("a migration restates the lists");
    let sql = fs::read_to_string(newest).expect("sql");
    let ceilings = check_list(&sql, "quota_ceilings_resource_valid");
    assert_eq!(ceilings, all(), "quota_ceilings in {}", newest.display());
    // `iteration_concurrency` is set through the tenant's collection policy, never a row of `quota_limits`
    let mut limits_expected = all();
    limits_expected.remove("iteration_concurrency");
    assert_eq!(
        check_list(&sql, "quota_limits_resource_valid"),
        limits_expected,
        "quota_limits in {}",
        newest.display()
    );
    let per_namespace: BTreeSet<String> = QuotaResource::ALL
        .iter()
        .filter(|r| r.scope() == QuotaScope::Namespace)
        .map(|r| r.as_str().to_owned())
        .collect();
    assert_eq!(
        check_list(&sql, "quota_limits_tenant_scoped"),
        per_namespace,
        "the resources a namespace may set, in {}",
        newest.display()
    );
}
