//! A digest of the content of every table, to prove that an operation left no row behind: two snapshots that are equal show that no
//! row of any table was inserted, deleted or changed (the audit trail, the receipts, the outbox, the counters of a schedule and a job
//! included). Shared by the fault suites (a crash leaves nothing) and the integration suites (a refusal leaves nothing).
#![allow(dead_code, clippy::expect_used)]

use std::collections::BTreeMap;

use sqlx::PgPool;

/// `table name -> md5 of the text of its rows`, for every table of the schema `public` except the migrations' bookkeeping and the
/// tables of the crash switch (`fault_*`, which a suite that installs it writes to on purpose).
pub async fn all_table_digests(pool: &PgPool) -> BTreeMap<String, String> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT tablename::text, \
                (xpath('/row/c/text()', query_to_xml( \
                    format('SELECT md5(coalesce(string_agg(t::text, '''' ORDER BY t::text), '''')) AS c FROM %I t', \
                           tablename), false, true, '')))[1]::text \
         FROM pg_tables \
         WHERE schemaname = 'public' AND tablename NOT LIKE 'fault\\_%' AND tablename <> '_sqlx_migrations'",
    )
    .fetch_all(pool)
    .await
    .expect("table digests")
    .into_iter()
    .collect()
}

/// The names of the tables whose digest is not the same in the two snapshots, for a message when a test fails.
#[must_use]
pub fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|table| before.get(*table) != after.get(*table))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
