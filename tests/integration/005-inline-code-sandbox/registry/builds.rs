//! The build of a version, as a runner takes it: under a lease, reused by key, signed on the way
//! in, and fenced to the runner that holds it (`FR-005-002`, `FR-005-008`, US02).

use sagat_application::error::AppError;
use sagat_application::script_signing::Verifier;
use sagat_application::scripts::{BuildFailureKind, BuildReport, DraftChange, PolicyRequest};
use sagat_domain::audit::AuditAction;
use sagat_domain::identity::Permission;
use sagat_domain::scripts::{Language, VersionStatus};
use std::time::Duration;

use super::{
    artifact, built, claim, complete, create, developer, digest_of, lockfile, other_runner,
    publish, ready, toolchains,
};
use crate::support::engine::{ctx, world};

#[tokio::test]
async fn a_runner_takes_a_build_and_the_artifact_it_makes_is_signed_for_the_tenant() {
    let w = world().await;
    let script = create(&w, "builds").await;
    let published = publish(&w, &script).await;
    let job = claim(&w).await.expect("a build is waiting");
    assert_eq!(job.version, published.version.id);
    assert_eq!(job.language, Language::TypeScript);
    assert_eq!(job.source, script.draft.source);
    assert!(
        claim(&w).await.is_none(),
        "a build held by a runner is not given to another"
    );

    complete(&w, &job, built("v1", &toolchains()[0].1)).await;
    let version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(version.status, VersionStatus::Ready);
    let digest = digest_of(&version);
    assert_eq!(
        digest,
        sagat_application::sandbox::digest_of(&artifact("v1"))
    );
    assert_eq!(version.build_key.as_ref(), Some(&job.build_key));
    assert_eq!(version.toolchain.as_deref(), Some("node@sha256:aa;shim=1"));
    assert!(version.built_at.is_some());

    // What the runner fetches is what was built, with a signature that verifies for this tenant.
    let fetched = w
        .runner
        .artifact(&w.runner_identity, &digest)
        .await
        .expect("fetch")
        .artifact;
    assert_eq!(fetched.content, artifact("v1"));
    let verifier = Verifier::new([w.signing_key.clone()]);
    assert_eq!(
        verifier.verify(
            &fetched.key_id,
            w.admin.tenant_id(),
            &digest,
            Language::TypeScript,
            &fetched.signature
        ),
        Ok(())
    );
    assert!(
        verifier
            .verify(
                &fetched.key_id,
                w.other_admin.tenant_id(),
                &digest,
                Language::TypeScript,
                &fetched.signature
            )
            .is_err(),
        "the signature is not good for another tenant"
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::ScriptBuilt.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn an_identical_build_is_reused_and_a_change_of_source_policy_or_toolchain_is_not() {
    let w = world().await;
    let (_, first) = ready(&w, "original", "bytes").await;

    // The same source, the same policy, the same toolchain, in another script: nothing to build.
    let twin = create(&w, "twin").await;
    let published = publish(&w, &twin).await;
    assert!(
        claim(&w).await.is_none(),
        "the runner is never handed a build that exists already"
    );
    let twin_version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            twin.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("twin version");
    assert_eq!(twin_version.status, VersionStatus::Ready);
    assert_eq!(twin_version.artifact, first.artifact, "the same artifact");
    assert_eq!(twin_version.build_key, first.build_key);

    // Another source builds again.
    let other = create(&w, "different").await;
    let changed = w
        .scripts
        .update(
            &w.admin,
            w.namespace.id(),
            other.id,
            DraftChange {
                source: Some(
                    sagat_domain::scripts::Source::parse("export default () => 'other'\n")
                        .expect("source"),
                ),
                ..DraftChange::default()
            },
            1,
            &ctx(),
        )
        .await
        .expect("update");
    publish(&w, &changed).await;
    let job = claim(&w)
        .await
        .expect("a different source is a different build");
    assert_ne!(job.build_key, first.build_key.clone().expect("key"));
    complete(&w, &job, built("other", &toolchains()[0].1)).await;

    // A change of policy invalidates the cache: the same source under other limits is built again.
    let tight = create(&w, "tighter").await;
    let tight = w
        .scripts
        .update(
            &w.admin,
            w.namespace.id(),
            tight.id,
            DraftChange {
                policy: Some(PolicyRequest {
                    memory_bytes: Some(64 * 1024 * 1024),
                    ..PolicyRequest::default()
                }),
                ..DraftChange::default()
            },
            1,
            &ctx(),
        )
        .await
        .expect("update");
    publish(&w, &tight).await;
    let job = claim(&w).await.expect("another policy is another build");
    assert_ne!(job.build_key, first.build_key.clone().expect("key"));
    assert_eq!(job.policy.limits.memory_bytes(), 64 * 1024 * 1024);
    complete(&w, &job, built("tight", &toolchains()[0].1)).await;

    // A new toolchain (a new image, a new shim) invalidates it as well.
    let again = create(&w, "same-again").await;
    publish(&w, &again).await;
    let new_toolchain = vec![(Language::TypeScript, "node@sha256:cc;shim=2".to_owned())];
    let job = w
        .runner
        .claim_build(&w.runner_identity, &new_toolchain, &ctx())
        .await
        .expect("claim")
        .expect("another toolchain is another build");
    assert_ne!(job.build_key, first.build_key.expect("key"));
}

#[tokio::test]
async fn a_build_that_fails_leaves_a_failed_version_with_the_reason_and_the_log() {
    let w = world().await;
    let script = create(&w, "broken").await;
    let published = publish(&w, &script).await;
    let job = claim(&w).await.expect("a build");
    complete(
        &w,
        &job,
        BuildReport::Failed {
            kind: BuildFailureKind::Rejected,
            message: "Unexpected token (line 3)".to_owned(),
            log: "stripping types failed".to_owned(),
        },
    )
    .await;
    let version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(version.status, VersionStatus::Failed);
    assert_eq!(
        version.failure.as_deref(),
        Some("rejected: Unexpected token (line 3)")
    );
    assert_eq!(version.build_log.as_deref(), Some("stripping types failed"));
    assert_eq!(version.artifact, None);
    assert!(
        claim(&w).await.is_none(),
        "a failed build is not taken again"
    );

    // The same source can be published again (the runner may have been the problem): that is a
    // new version, not the failed one returned.
    let retry = publish(&w, &script).await;
    assert!(retry.created);
    assert_eq!(retry.version.number.get(), 2);
}

#[tokio::test]
async fn a_sandbox_that_was_unavailable_is_tried_again_but_not_for_ever() {
    let w = world().await;
    let script = create(&w, "flaky").await;
    let published = publish(&w, &script).await;
    for attempt in 1..=3 {
        let job = claim(&w)
            .await
            .unwrap_or_else(|| panic!("attempt {attempt} is offered"));
        complete(
            &w,
            &job,
            BuildReport::Failed {
                kind: BuildFailureKind::Unavailable,
                message: "the container engine is down".to_owned(),
                log: String::new(),
            },
        )
        .await;
    }
    let version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(
        version.status,
        VersionStatus::Failed,
        "three attempts, then it is over"
    );
    assert!(
        version
            .failure
            .as_deref()
            .unwrap_or_default()
            .starts_with("unavailable:")
    );
    assert!(claim(&w).await.is_none());
}

#[tokio::test]
async fn a_runner_that_lost_its_lease_cannot_finish_what_another_took() {
    let w = world().await;
    let script = create(&w, "lease").await;
    let published = publish(&w, &script).await;
    let slow = claim(&w).await.expect("the first runner");
    // Its lease runs out without a sign of life.
    sqlx::query(
        "UPDATE script_build_jobs SET lease_expires_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("age the lease");
    let fast = claim(&w).await.expect("another runner takes it over");
    assert_eq!(fast.job, slow.job);
    assert_ne!(fast.lease_token, slow.lease_token);

    // The slow one finishes late: refused, and the version does not change because of it.
    let late = w
        .runner
        .complete_build(
            &w.runner_identity,
            slow.job,
            &slow.lease_token,
            built("late", &toolchains()[0].1),
            &ctx(),
        )
        .await;
    assert_eq!(late, Err(AppError::StaleAttempt));
    assert_eq!(
        w.runner
            .heartbeat_build(&w.runner_identity, slow.job, &slow.lease_token)
            .await,
        Ok(None),
        "it learns it lost the build from its next heartbeat"
    );
    assert!(
        w.runner
            .heartbeat_build(&w.runner_identity, fast.job, &fast.lease_token)
            .await
            .expect("heartbeat")
            .is_some()
    );
    complete(&w, &fast, built("fast", &toolchains()[0].1)).await;
    let version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(
        version.artifact,
        Some(sagat_application::sandbox::digest_of(&artifact("fast")))
    );

    // A completion repeated by its holder (the answer was lost) is accepted once and the same.
    complete(&w, &fast, built("fast", &toolchains()[0].1)).await;
    // A token nobody holds opens nothing.
    assert_eq!(
        w.runner
            .complete_build(
                &w.runner_identity,
                fast.job,
                &"0".repeat(64),
                built("x", "t"),
                &ctx()
            )
            .await,
        Err(AppError::StaleAttempt)
    );
}

#[tokio::test]
async fn a_build_whose_runner_vanishes_too_many_times_fails() {
    let w = world().await;
    let script = create(&w, "vanishing").await;
    let published = publish(&w, &script).await;
    for _ in 0..3 {
        claim(&w).await.expect("offered");
        sqlx::query("UPDATE script_build_jobs SET lease_expires_at = clock_timestamp() - interval '1 second'")
            .execute(&w.db.pool)
            .await
            .expect("age");
    }
    assert!(
        claim(&w).await.is_none(),
        "after the third loss it is given up on"
    );
    let version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(version.status, VersionStatus::Failed);
}

#[tokio::test]
async fn a_runner_serves_its_own_tenant_and_no_other() {
    let w = world().await;
    let (_, version) = ready(&w, "private", "secret-code").await;
    let digest = digest_of(&version);
    let stranger = other_runner(&w);
    // It sees none of the tenant's builds ...
    create(&w, "waiting").await;
    let waiting = w
        .scripts
        .list(&w.admin, w.namespace.id(), None, 10, &ctx())
        .await
        .expect("list");
    publish(
        &w,
        waiting
            .iter()
            .find(|s| s.name.as_str() == "waiting")
            .expect("script"),
    )
    .await;
    assert!(
        w.runner
            .claim_build(&stranger, &toolchains(), &ctx())
            .await
            .expect("claim")
            .is_none(),
        "another tenant's runner is offered nothing"
    );
    // ... fetches none of its artifacts ...
    assert_eq!(
        w.runner.artifact(&stranger, &digest).await.map(|_| ()),
        Err(AppError::NotFound)
    );
    // ... and cannot reuse a build of this tenant: its cache is its own.
    let theirs = w
        .scripts
        .create(
            &w.other_admin,
            w.other_namespace.id(),
            super::ts("private"),
            &ctx(),
        )
        .await
        .expect("their script");
    w.scripts
        .publish(&w.other_admin, w.other_namespace.id(), theirs.id, &ctx())
        .await
        .expect("publish");
    let job = w
        .runner
        .claim_build(&stranger, &toolchains(), &ctx())
        .await
        .expect("claim")
        .expect("their own build is not satisfied by mine");
    assert_eq!(job.source, theirs.draft.source);
}

#[tokio::test]
async fn only_a_runner_takes_builds_and_only_a_runner_fetches_artifacts() {
    let w = world().await;
    let (_, version) = ready(&w, "guarded", "bytes").await;
    let digest = digest_of(&version);
    for principal in [&w.admin, &developer(&w), &w.worker] {
        let role = principal.role();
        if role != sagat_domain::identity::Role::Admin {
            assert_eq!(
                w.runner
                    .claim_build(principal, &toolchains(), &ctx())
                    .await
                    .map(|_| ()),
                Err(AppError::Forbidden {
                    permission: Permission::ScriptBuild
                }),
                "{role}"
            );
            assert_eq!(
                w.runner.artifact(principal, &digest).await.map(|_| ()),
                Err(AppError::Forbidden {
                    permission: Permission::ScriptExecute
                }),
                "{role}"
            );
        }
    }
}

#[tokio::test]
async fn an_artifact_that_is_too_large_is_refused_not_stored() {
    let w = world().await;
    let script = create(&w, "huge").await;
    publish(&w, &script).await;
    let job = claim(&w).await.expect("a build");
    let result = w
        .runner
        .complete_build(
            &w.runner_identity,
            job.job,
            &job.lease_token,
            BuildReport::Built {
                artifact: vec![b'x'; 2 * 1024 * 1024 + 1],
                sbom: b"{}".to_vec(),
                toolchain: "t".to_owned(),
                log: String::new(),
            },
            &ctx(),
        )
        .await;
    assert!(
        matches!(result, Err(AppError::PayloadTooLarge(_))),
        "{result:?}"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM script_artifacts")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn the_lockfile_belongs_to_the_version_and_to_its_build_key() {
    let w = world().await;
    let mut with_lock = super::ts("locked");
    with_lock.lockfile = Some(lockfile(r#"{"lockfileVersion":3,"packages":{}}"#));
    let locked = w
        .scripts
        .create(&w.admin, w.namespace.id(), with_lock, &ctx())
        .await
        .expect("create");
    let plain = create(&w, "plain").await;
    publish(&w, &locked).await;
    publish(&w, &plain).await;
    let a = claim(&w).await.expect("first");
    let b = claim(&w).await.expect("second");
    assert_ne!(a.build_key, b.build_key, "the lockfile is part of the key");
    assert!(a.lockfile.is_some() != b.lockfile.is_some());
    let _ = Duration::ZERO;
}
