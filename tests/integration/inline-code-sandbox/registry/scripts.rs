//! Scripts and their drafts: who may write them, what is refused, and that nothing leaks across
//! tenants or past a limit (`FR-005-001`).

use std::time::Duration;

use sagat_application::error::{AppError, ConflictKind};
use sagat_application::scripts::{DraftChange, PolicyRequest};
use sagat_domain::audit::AuditAction;
use sagat_domain::identity::Permission;
use sagat_domain::quota::QuotaResource;
use sagat_domain::scripts::{Egress, Language, ScriptError, Source, VersionStatus};

use super::{
    PY_SOURCE, TS_SOURCE, built, claim, complete, create, developer, publish, ready, spec, ts,
    viewer,
};
use crate::support::engine::{ctx, world};

#[tokio::test]
async fn an_author_writes_reads_changes_and_deletes_a_script() {
    let w = world().await;
    let script = create(&w, "double-it").await;
    assert_eq!(script.revision, 1);
    assert_eq!(script.latest_version, None);
    assert_eq!(script.draft.source.as_str(), TS_SOURCE);

    let listed = w
        .scripts
        .list(&w.admin, w.namespace.id(), None, 50, &ctx())
        .await
        .expect("list");
    assert_eq!(
        listed.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![script.id]
    );
    assert_eq!(
        listed[0].source_bytes,
        TS_SOURCE.len() as u64,
        "a listing says how big the source is and does not carry it"
    );

    let changed = w
        .scripts
        .update(
            &w.admin,
            w.namespace.id(),
            script.id,
            DraftChange {
                source: Some(Source::parse("export default () => 2\n").expect("source")),
                ..DraftChange::default()
            },
            1,
            &ctx(),
        )
        .await
        .expect("update");
    assert_eq!(changed.revision, 2);
    assert_eq!(changed.draft.source.as_str(), "export default () => 2\n");
    assert!(changed.updated_at >= script.updated_at);

    w.scripts
        .delete(&w.admin, w.namespace.id(), script.id, &ctx())
        .await
        .expect("delete");
    assert_eq!(
        w.scripts
            .get(&w.admin, w.namespace.id(), script.id, &ctx())
            .await,
        Err(AppError::NotFound)
    );
}

#[tokio::test]
async fn two_editors_never_silently_overwrite_each_other() {
    let w = world().await;
    let script = create(&w, "shared").await;
    let edit = |n: u32| DraftChange {
        source: Some(Source::parse(&format!("export default () => {n}\n")).expect("source")),
        ..DraftChange::default()
    };
    w.scripts
        .update(&w.admin, w.namespace.id(), script.id, edit(1), 1, &ctx())
        .await
        .expect("first");
    // The second editor read revision 1 as well.
    assert_eq!(
        w.scripts
            .update(&w.admin, w.namespace.id(), script.id, edit(2), 1, &ctx())
            .await,
        Err(AppError::RevisionConflict { current: 2 })
    );
    let kept = w
        .scripts
        .get(&w.admin, w.namespace.id(), script.id, &ctx())
        .await
        .expect("get");
    assert_eq!(kept.draft.source.as_str(), "export default () => 1\n");
}

#[tokio::test]
async fn a_name_is_taken_once_per_namespace_and_a_script_count_is_a_hard_limit() {
    let w = world().await;
    create(&w, "taken").await;
    assert_eq!(
        w.scripts
            .create(&w.admin, w.namespace.id(), ts("taken"), &ctx())
            .await,
        Err(AppError::Conflict(ConflictKind::ScriptName))
    );
    // The same name in the other tenant's namespace is another script.
    w.scripts
        .create(&w.other_admin, w.other_namespace.id(), ts("taken"), &ctx())
        .await
        .expect("a name is per namespace");

    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::Scripts,
            2,
            &ctx(),
        )
        .await
        .expect("set the limit");
    create(&w, "second").await;
    assert!(matches!(
        w.scripts
            .create(&w.admin, w.namespace.id(), ts("third"), &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
}

#[tokio::test]
async fn only_a_script_that_can_be_built_is_accepted() {
    let w = world().await;
    w.scripts
        .create(
            &w.admin,
            w.namespace.id(),
            spec("csharp", Language::CSharp, "public sealed class Script {}"),
            &ctx(),
        )
        .await
        .expect("C# is built");
    for language in [Language::Go, Language::Java] {
        w.scripts
            .create(
                &w.admin,
                w.namespace.id(),
                spec(&format!("{}-script", language.as_str()), language, "x"),
                &ctx(),
            )
            .await
            .expect("dedicated builder supported");
    }
    let mut bad_schema = ts("bad-schema");
    bad_schema.input_schema = Some(br#"{"type": 12}"#.to_vec());
    assert_eq!(
        w.scripts
            .create(&w.admin, w.namespace.id(), bad_schema, &ctx())
            .await,
        Err(AppError::InvalidScript(ScriptError::InvalidSchema))
    );
    let mut not_json = ts("not-json");
    not_json.output_schema = Some(b"{nope".to_vec());
    assert_eq!(
        w.scripts
            .create(&w.admin, w.namespace.id(), not_json, &ctx())
            .await,
        Err(AppError::InvalidScript(ScriptError::InvalidSchema))
    );
    let mut remote_ref = ts("remote-ref");
    remote_ref.input_schema = Some(br#"{"$ref": "https://example.com/schema.json"}"#.to_vec());
    assert_eq!(
        w.scripts
            .create(&w.admin, w.namespace.id(), remote_ref, &ctx())
            .await,
        Err(AppError::InvalidScript(ScriptError::InvalidSchema)),
        "a reference to anything outside the document is refused, never fetched"
    );
    let mut good = spec("python-ok", Language::Python, PY_SOURCE);
    good.input_schema = Some(
        br#"{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}"#.to_vec(),
    );
    w.scripts
        .create(&w.admin, w.namespace.id(), good, &ctx())
        .await
        .expect("a valid schema");
}

#[tokio::test]
async fn a_script_cannot_ask_for_more_than_the_platform_allows() {
    let w = world().await;
    for request in [
        PolicyRequest {
            memory_bytes: Some(1024 * 1024 * 1024),
            ..PolicyRequest::default()
        },
        PolicyRequest {
            wall: Some(Duration::from_secs(600)),
            ..PolicyRequest::default()
        },
        PolicyRequest {
            egress: Some(Egress::Namespace),
            ..PolicyRequest::default()
        },
    ] {
        let mut asking = ts("greedy");
        asking.policy = request;
        assert_eq!(
            w.scripts
                .create(&w.admin, w.namespace.id(), asking, &ctx())
                .await,
            Err(AppError::InvalidScript(ScriptError::AboveCeiling)),
            "{request:?}"
        );
    }
    let mut modest = ts("modest");
    modest.policy = PolicyRequest {
        memory_bytes: Some(64 * 1024 * 1024),
        ..PolicyRequest::default()
    };
    let script = w
        .scripts
        .create(&w.admin, w.namespace.id(), modest, &ctx())
        .await
        .expect("less is fine");
    assert_eq!(script.draft.policy.limits.memory_bytes(), 64 * 1024 * 1024);
}

#[tokio::test]
async fn permissions_follow_the_roles() {
    let w = world().await;
    let dev = developer(&w);
    let view = viewer(&w);
    let script = w
        .scripts
        .create(&dev, w.namespace.id(), ts("by-dev"), &ctx())
        .await
        .expect("a developer writes");
    // A viewer reads and nothing else.
    assert!(
        w.scripts
            .get(&view, w.namespace.id(), script.id, &ctx())
            .await
            .is_ok()
    );
    assert_eq!(
        w.scripts
            .create(&view, w.namespace.id(), ts("by-viewer"), &ctx())
            .await,
        Err(AppError::Forbidden {
            permission: Permission::ScriptWrite
        })
    );
    assert_eq!(
        w.scripts
            .publish(&view, w.namespace.id(), script.id, &ctx())
            .await
            .map(|p| p.created),
        Err(AppError::Forbidden {
            permission: Permission::ScriptPublish
        })
    );
    // The runner's identity reads, writes and publishes nothing.
    for result in [
        w.scripts
            .get(&w.runner_identity, w.namespace.id(), script.id, &ctx())
            .await
            .map(|_| ()),
        w.scripts
            .create(
                &w.runner_identity,
                w.namespace.id(),
                ts("by-runner"),
                &ctx(),
            )
            .await
            .map(|_| ()),
        w.scripts
            .publish(&w.runner_identity, w.namespace.id(), script.id, &ctx())
            .await
            .map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(AppError::Forbidden { .. })),
            "{result:?}"
        );
    }
    // A worker that serves activities does not read scripts either.
    assert!(matches!(
        w.scripts
            .get(&w.worker, w.namespace.id(), script.id, &ctx())
            .await,
        Err(AppError::Forbidden { .. })
    ));
}

#[tokio::test]
async fn another_tenants_script_is_not_found_and_the_probe_is_audited() {
    let w = world().await;
    let script = create(&w, "mine").await;
    // The other tenant addresses my namespace and my script.
    assert_eq!(
        w.scripts
            .get(&w.other_admin, w.namespace.id(), script.id, &ctx())
            .await,
        Err(AppError::NotFound)
    );
    // And its own namespace with my script's id: also nothing.
    assert_eq!(
        w.scripts
            .get(&w.other_admin, w.other_namespace.id(), script.id, &ctx())
            .await,
        Err(AppError::NotFound)
    );
    // The probe is written off the response path, against the tenant that made it.
    let probes = crate::support::eventually("the probe is audited", || async {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT resource_id FROM audit_records WHERE tenant_id = $1 AND action = $2",
        )
        .bind(w.other_admin.tenant_id().as_uuid())
        .bind(AuditAction::CrossTenantAccessDenied.as_str())
        .fetch_all(&w.db.pool)
        .await
        .expect("audit");
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert_eq!(
        probes,
        vec![w.namespace.id().to_string()],
        "naming the namespace they asked for"
    );
    assert!(
        w.scripts
            .list(&w.other_admin, w.other_namespace.id(), None, 50, &ctx())
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn publishing_makes_an_immutable_version_and_the_same_draft_is_the_same_version() {
    let w = world().await;
    let script = create(&w, "versioned").await;
    let first = publish(&w, &script).await;
    assert!(first.created);
    assert_eq!(first.version.number.get(), 1);
    assert_eq!(first.version.status, VersionStatus::Building);
    assert_eq!(first.version.draft, script.draft);

    // Nothing changed: not another version.
    let again = publish(&w, &script).await;
    assert!(!again.created);
    assert_eq!(again.version.id, first.version.id);

    // A change is another one, and the first keeps what it had.
    let changed = w
        .scripts
        .update(
            &w.admin,
            w.namespace.id(),
            script.id,
            DraftChange {
                source: Some(Source::parse("export default () => 3\n").expect("source")),
                ..DraftChange::default()
            },
            1,
            &ctx(),
        )
        .await
        .expect("update");
    let second = w
        .scripts
        .publish(&w.admin, w.namespace.id(), changed.id, &ctx())
        .await
        .expect("publish");
    assert!(second.created);
    assert_eq!(second.version.number.get(), 2);
    let first_again = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            first.version.number,
            &ctx(),
        )
        .await
        .expect("version 1");
    assert_eq!(
        first_again.draft.source.as_str(),
        TS_SOURCE,
        "version 1 is what was published"
    );
    let versions = w
        .scripts
        .versions(&w.admin, w.namespace.id(), script.id, None, 10, &ctx())
        .await
        .expect("versions");
    assert_eq!(
        versions.iter().map(|v| v.number.get()).collect::<Vec<_>>(),
        vec![2, 1]
    );
    let script_now = w
        .scripts
        .get(&w.admin, w.namespace.id(), script.id, &ctx())
        .await
        .expect("get");
    assert_eq!(script_now.latest_version.map(|n| n.get()), Some(2));
}

#[tokio::test]
async fn the_database_itself_refuses_to_change_or_remove_a_published_version() {
    let w = world().await;
    let (script, version) = ready(&w, "frozen", "frozen-bytes").await;
    for statement in [
        "UPDATE script_versions SET source = 'export default () => 666' WHERE id = $1",
        "UPDATE script_versions SET policy = 'v1;memory=1' WHERE id = $1",
        "UPDATE script_versions SET status = 'FAILED', failure = 'x' WHERE id = $1",
        "UPDATE script_versions SET artifact_digest = NULL, status = 'BUILDING' WHERE id = $1",
        "DELETE FROM script_versions WHERE id = $1",
    ] {
        let refused = sqlx::query(statement)
            .bind(version.id.as_uuid())
            .execute(&w.db.pool)
            .await;
        assert!(refused.is_err(), "{statement} must be refused");
    }
    let refused = sqlx::query("TRUNCATE script_versions CASCADE")
        .execute(&w.db.pool)
        .await;
    assert!(refused.is_err(), "truncate must be refused");
    // Nor can the artifact a version names be changed or removed.
    let digest = version.artifact.clone().expect("artifact");
    for statement in [
        "UPDATE script_artifacts SET content = 'other'::bytea WHERE digest = $1",
        "DELETE FROM script_artifacts WHERE digest = $1",
    ] {
        let refused = sqlx::query(statement)
            .bind(digest.as_str())
            .execute(&w.db.pool)
            .await;
        assert!(refused.is_err(), "{statement} must be refused");
    }
    // A version whose build is still going can only learn how it ended: its source stays.
    let pending = create(&w, "pending-one").await;
    let published = publish(&w, &pending).await;
    let refused =
        sqlx::query("UPDATE script_versions SET source = 'export default () => 1' WHERE id = $1")
            .bind(published.version.id.as_uuid())
            .execute(&w.db.pool)
            .await;
    assert!(
        refused.is_err(),
        "a version being built keeps its source too"
    );
    let _ = script;
}

#[tokio::test]
async fn a_script_with_a_version_cannot_be_deleted_because_a_workflow_may_pin_it() {
    let w = world().await;
    let script = create(&w, "pinned").await;
    publish(&w, &script).await;
    assert_eq!(
        w.scripts
            .delete(&w.admin, w.namespace.id(), script.id, &ctx())
            .await,
        Err(AppError::StateConflict(
            "a script with a published version cannot be deleted"
        ))
    );
    assert!(
        w.scripts
            .get(&w.admin, w.namespace.id(), script.id, &ctx())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn publishing_is_refused_when_the_draft_moved_on_after_it_was_read() {
    use sagat_application::scripts::{NewVersion, ScriptStore};
    use sagat_domain::scripts::{BuildJobId, ScriptVersionId};
    let w = world().await;
    let script = create(&w, "racing").await;
    // Someone changes the draft between the moment the publisher read it and the moment it commits.
    w.scripts
        .update(
            &w.admin,
            w.namespace.id(),
            script.id,
            DraftChange {
                source: Some(Source::parse("export default () => 9\n").expect("source")),
                ..DraftChange::default()
            },
            1,
            &ctx(),
        )
        .await
        .expect("update");
    let stale = NewVersion {
        draft_revision: 1,
        id: ScriptVersionId::generate(),
        job_id: BuildJobId::generate(),
        script_id: script.id,
        tenant_id: script.tenant_id,
        namespace_id: script.namespace_id,
        language: script.language,
        content_digest: script.draft.content_digest(script.language),
        draft: script.draft.clone(),
        created_by: "dev".to_owned(),
        created_at: crate::support::now(),
    };
    let audit = sagat_domain::audit::AuditRecord::by(
        &w.admin,
        AuditAction::ScriptPublished,
        sagat_domain::audit::ResourceType::ScriptVersion,
        stale.id.to_string(),
        None,
        crate::support::now(),
    );
    let outcome = w.store.publish_version(&stale, 100, &audit).await;
    assert!(
        matches!(
            outcome,
            Err(sagat_application::error::StoreError::RevisionConflict { current: 2 })
        ),
        "{outcome:?}"
    );
    let versions = w
        .scripts
        .versions(&w.admin, w.namespace.id(), script.id, None, 10, &ctx())
        .await
        .expect("versions");
    assert!(
        versions.is_empty(),
        "nothing was published from the stale draft"
    );
}

#[tokio::test]
async fn builds_in_progress_are_a_hard_limit_of_the_namespace() {
    let w = world().await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ScriptJobs,
            2,
            &ctx(),
        )
        .await
        .expect("set the limit");
    let (a, b, c) = (
        create(&w, "job-a").await,
        create(&w, "job-b").await,
        create(&w, "job-c").await,
    );
    publish(&w, &a).await;
    publish(&w, &b).await;
    assert!(matches!(
        w.scripts
            .publish(&w.admin, w.namespace.id(), c.id, &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    // A test run counts as work in progress too.
    assert!(matches!(
        w.scripts
            .test(&w.admin, w.namespace.id(), c.id, b"{}".to_vec(), &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    // Finishing a build makes room.
    let job = claim(&w).await.expect("a build");
    complete(&w, &job, built(&job, "job-a")).await;
    w.scripts
        .publish(&w.admin, w.namespace.id(), c.id, &ctx())
        .await
        .expect("room again");
}

#[tokio::test]
async fn a_script_has_a_bounded_number_of_versions_because_they_are_never_deleted() {
    let w = world().await;
    let script = create(&w, "many").await;
    // 499 versions that failed to build, as if published one after the other.
    sqlx::query(
        "INSERT INTO script_versions (id, tenant_id, namespace_id, script_id, number, language, \
             status, source, policy, retention_days, content_digest, failure, created_by, created_at) \
         SELECT gen_random_uuid(), s.tenant_id, s.namespace_id, s.id, n, s.language, 'FAILED', 'x', \
                s.policy, s.retention_days, 'sha256:' || lpad(to_hex(n), 64, '0'), \
                'unavailable: seeded', 'test', now() \
         FROM scripts s, generate_series(1, 499) n WHERE s.id = $1",
    )
    .bind(script.id.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("seed the versions");
    sqlx::query("UPDATE scripts SET latest_version = 499 WHERE id = $1")
        .bind(script.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("seed the counter");
    let edit = |n: u32, revision: u64| {
        let w = &w;
        async move {
            w.scripts
                .update(
                    &w.admin,
                    w.namespace.id(),
                    script.id,
                    DraftChange {
                        source: Some(
                            Source::parse(&format!("export default () => {n}\n")).expect("source"),
                        ),
                        ..DraftChange::default()
                    },
                    revision,
                    &ctx(),
                )
                .await
                .expect("update");
        }
    };
    edit(1, 1).await;
    let last = w
        .scripts
        .publish(&w.admin, w.namespace.id(), script.id, &ctx())
        .await
        .expect("the 500th version is allowed");
    assert_eq!(last.version.number.get(), 500);
    edit(2, 2).await;
    assert_eq!(
        w.scripts
            .publish(&w.admin, w.namespace.id(), script.id, &ctx())
            .await
            .map(|p| p.version.number.get()),
        Err(AppError::QuotaExceeded("versions of one script")),
        "the 501st is refused: what one script can keep is bounded"
    );
}

#[tokio::test]
async fn publishing_and_trying_the_same_script_at_the_same_time_never_wait_for_each_other_for_ever()
{
    // A stress, not a proof: publishing holds the script's row and then the namespace's lock, and
    // trying takes the same two, so they must take them in the same order. Run together many times,
    // a pair that took them the other way round would deadlock and one side would fail (40P01).
    let w = world().await;
    let script = create(&w, "contended").await;
    for (n, revision) in (0..25_u32).zip(1_u64..) {
        w.scripts
            .update(
                &w.admin,
                w.namespace.id(),
                script.id,
                DraftChange {
                    source: Some(
                        Source::parse(&format!("export default () => {n}\n")).expect("source"),
                    ),
                    ..DraftChange::default()
                },
                revision,
                &ctx(),
            )
            .await
            .expect("update");
        let context = ctx();
        let publish = w
            .scripts
            .publish(&w.admin, w.namespace.id(), script.id, &context);
        let test = w.scripts.test(
            &w.admin,
            w.namespace.id(),
            script.id,
            b"{}".to_vec(),
            &context,
        );
        let (published, tried) = tokio::join!(publish, test);
        published.expect("publish");
        tried.expect("test");
        // Keep the namespace's quota free for the next round.
        sqlx::query("UPDATE script_build_jobs SET status = 'FAILED', finished_at = now()")
            .execute(&w.db.pool)
            .await
            .expect("clear");
        sqlx::query("UPDATE script_test_runs SET status = 'FAILED', failure = 'SCRIPT_ERROR', finished_at = now()")
            .execute(&w.db.pool)
            .await
            .expect("clear");
    }
}

#[tokio::test]
async fn durable_retention_ceiling_guards_new_script_choices_publication_and_tests() {
    use sagat_domain::scripts::Retention;
    let w = world().await;
    let existing = create(&w, "retention-before").await;
    let published = publish(&w, &existing).await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ScriptRetentionDays,
            1,
            &ctx(),
        )
        .await
        .expect("lower ceiling");
    assert!(matches!(
        w.scripts
            .create(&w.admin, w.namespace.id(), ts("too-long"), &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    assert!(matches!(
        w.scripts
            .test(
                &w.admin,
                w.namespace.id(),
                existing.id,
                b"{}".to_vec(),
                &ctx()
            )
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    // Exact publication replay remains the existing version, even after policy lowering.
    let replay = publish(&w, &existing).await;
    assert_eq!(published.version.id, replay.version.id);
    let changed = w
        .scripts
        .update(
            &w.admin,
            w.namespace.id(),
            existing.id,
            DraftChange {
                retention: Some(Retention::days(2).expect("days")),
                ..DraftChange::default()
            },
            existing.revision,
            &ctx(),
        )
        .await;
    assert!(matches!(changed, Err(AppError::QuotaExceeded(_))));
    let bounded = w
        .scripts
        .update(
            &w.admin,
            w.namespace.id(),
            existing.id,
            DraftChange {
                retention: Some(Retention::days(1).expect("days")),
                ..DraftChange::default()
            },
            existing.revision,
            &ctx(),
        )
        .await
        .expect("bounded choice");
    w.scripts
        .test(
            &w.admin,
            w.namespace.id(),
            bounded.id,
            b"{}".to_vec(),
            &ctx(),
        )
        .await
        .expect("bounded new test");
    let old: i16 = sqlx::query_scalar("SELECT retention_days FROM script_versions WHERE id=$1")
        .bind(published.version.id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("immutable retention");
    assert_eq!(old, existing.draft.retention.get() as i16);
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ScriptRetentionDays,
            0,
            &ctx(),
        )
        .await
        .expect("disable new retention");
    assert!(matches!(
        w.scripts
            .publish(&w.admin, w.namespace.id(), bounded.id, &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
}
