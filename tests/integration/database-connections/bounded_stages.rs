//! A stall in this suite fails with the name of its stage instead of consuming the CI budget.
//!
//! Pull request 45 hung for six hours in the managed secret lifecycle: the task lease (30 s) expired
//! on a slow runner before the last scenario, the call was refused before it read the provider, and a
//! barrier waiting for that read never opened. These tests keep that shape from coming back.
use super::{ReadBarrier, revoke_during_provider_read, world};
use crate::support::{
    bounded,
    engine::{activity, document, with},
};
use sagat_application::secret_provider::{ProviderFuture, ProviderReference, SecretProvider};
use sagat_domain::secrets::SecretValue;
use serde_json::json;
use std::{future::pending, sync::Arc, time::Duration};

#[tokio::test]
#[should_panic(expected = "stage `pull and start example` did not finish within 50ms")]
async fn a_stage_that_does_not_finish_is_named_with_its_limit() {
    bounded::within(
        "pull and start example",
        Duration::from_millis(50),
        pending::<()>(),
    )
    .await;
}

#[tokio::test]
#[should_panic(
    expected = "scenario `demo` did not finish within 50ms; it was in stage `waiting on a barrier`"
)]
async fn a_scenario_that_stalls_names_the_stage_it_was_in() {
    bounded::lifecycle("demo", Duration::from_millis(50), |progress| async move {
        progress.enter("first");
        progress.enter("waiting on a barrier");
        pending::<()>().await;
    })
    .await;
}

#[tokio::test]
async fn a_scenario_that_finishes_returns_its_value() {
    let value = bounded::lifecycle("demo", Duration::from_secs(5), |progress| async move {
        progress.enter("only");
        7
    })
    .await;
    assert_eq!(value, 7);
}

/// A provider that fails the test if anything reads it.
struct Unreachable;
impl SecretProvider for Unreachable {
    fn create<'a>(
        &'a self,
        _: &'a ProviderReference,
        _: &'a SecretValue,
    ) -> ProviderFuture<'a, ()> {
        unreachable!("no write is expected")
    }
    fn read<'a>(&'a self, _: &'a ProviderReference) -> ProviderFuture<'a, Option<SecretValue>> {
        unreachable!("an expired lease must be refused before the provider is read")
    }
    fn delete<'a>(&'a self, _: &'a ProviderReference) -> ProviderFuture<'a, ()> {
        unreachable!("no delete is expected")
    }
}

/// The cause of the hang: the lease is gone when the call is made, so no read reaches the barrier.
#[tokio::test]
#[should_panic(expected = "resolve_secrets returned before it read the provider")]
async fn a_lease_that_expired_before_the_provider_read_fails_instead_of_waiting_for_a_read() {
    // Without the guard in `revoke_during_provider_read` this ends at the 60 s limit below, naming it.
    bounded::within(
        "expired lease before the provider read",
        Duration::from_secs(60),
        async {
            let mut w = world().await;
            w.put_secret("db-credentials", "expired-lease-canary").await;
            let (workflow, _) = w
                .try_publish_as(
                    &w.admin,
                    document(
                        "expired-lease",
                        "a",
                        vec![with(activity("a"), "secrets", json!(["db-credentials"]))],
                    ),
                )
                .await
                .unwrap();
            let run = w.start(workflow, &json!({})).await;
            let session = w.session(&["act.a"]).await;
            let task = w.one_task(&session).await;
            w.age_lease(run, "a").await;
            let gated = Arc::new(ReadBarrier::new(Arc::new(Unreachable)));
            w.workers = w.workers.with_secret_provider(gated.clone());
            revoke_during_provider_read(&w, &gated, &session, &task).await;
        },
    )
    .await;
}
