//! Design 043, SC-043-004, through real starts: the compiled-schema cache of a process compiles a schema once for 1,000
//! starts of one version, once for two versions of one tenant with byte-identical schemas, twice for the same schema in two
//! tenants, and remembers one failure for 1,000 starts of a version whose schema is outside the subset. One test in a binary
//! of its own: the counters are the process's, so nothing else may start runs here.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

use sagat_application::definitions::InputValidationMode;
use sagat_infra_schema::run_input::RunInputValidator;
use serde_json::json;

use support::engine::{activity, document, world};
use support::run_input::{order_schema, publish_in, unusable_schema, with_schema};

#[tokio::test]
async fn the_cache_compiles_once_per_tenant_and_schema_and_remembers_a_failure_once() {
    let w = world().await;
    let stats = || RunInputValidator::process().schemas().cache_stats();
    let (one, _) = w
        .publish(with_schema(
            document("cache-one", "a", vec![activity("a")]),
            order_schema(),
        ))
        .await;
    let before = stats();
    for _ in 0..1_000 {
        w.start(one, &json!({"orderId": "o-1"})).await;
    }
    let after = stats();
    assert_eq!(
        after.misses - before.misses,
        1,
        "one compile for 1,000 starts of one version"
    );
    assert_eq!(after.hits - before.hits, 999);

    // a second version with byte-identical schema, same tenant: the entry is shared
    let (two, _) = w
        .publish(with_schema(
            document("cache-two", "a", vec![activity("a")]),
            order_schema(),
        ))
        .await;
    let before = stats();
    w.start(two, &json!({"orderId": "o-1"})).await;
    assert_eq!(
        stats().misses - before.misses,
        0,
        "byte-identical schemas of one tenant share a compile"
    );

    // the same schema in the other tenant compiles again
    let other = w
        .publish_as_other_tenant(with_schema(
            document("cache-three", "a", vec![activity("a")]),
            order_schema(),
        ))
        .await;
    let before = stats();
    w.start_as_other_tenant(other, &json!({"orderId": "o-1"}))
        .await
        .expect("started");
    assert_eq!(
        stats().misses - before.misses,
        1,
        "two tenants never share a compiled schema"
    );

    // a schema outside the subset: one failure remembered for 1,000 refused starts
    let (bad, _) = publish_in(
        &w,
        InputValidationMode::Report,
        &with_schema(
            document("cache-bad", "a", vec![activity("a")]),
            unusable_schema(),
        ),
    )
    .await;
    let before = stats();
    for _ in 0..1_000 {
        w.start_with(
            bad,
            &json!({}),
            &format!("start-{}", uuid::Uuid::now_v7()),
            None,
        )
        .await
        .expect_err("refused");
    }
    let after = stats();
    assert_eq!(
        after.failures - before.failures,
        1,
        "one remembered failure"
    );
    assert_eq!(
        after.misses - before.misses,
        1,
        "compiled, and refused, once"
    );
}
