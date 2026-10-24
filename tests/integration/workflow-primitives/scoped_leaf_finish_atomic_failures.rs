//! Bounded confirmed-failure atomicity for dormant flat OrdinaryLeafV1 Finish.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. Every failure case owns an isolated
//! database, uses genuine public flat source plus native certification, and invokes the adapter
//! through an explicit `sagat_app` pool. The sequence is a deliberately nontransactional,
//! owner-only test marker proving that the named trigger was reached; it is not product state.

use super::*;
use sagat_application::error::StoreError;
use std::time::Duration as StdDuration;
use tokio::time::timeout;

const ATTEMPT_DEADLINE: StdDuration = StdDuration::from_secs(30);

#[derive(Clone, Copy, Debug)]
enum FailureStage {
    Audit,
    Outbox,
    Receipt,
    DeferredCommit,
}

impl FailureStage {
    const ALL: [Self; 4] = [
        Self::Audit,
        Self::Outbox,
        Self::Receipt,
        Self::DeferredCommit,
    ];

    fn slug(self) -> &'static str {
        match self {
            Self::Audit => "audit",
            Self::Outbox => "outbox",
            Self::Receipt => "receipt",
            Self::DeferredCommit => "deferred_commit",
        }
    }

    fn relation(self) -> &'static str {
        match self {
            Self::Audit => "audit_records",
            Self::Outbox => "event_outbox",
            Self::Receipt | Self::DeferredCommit => "capacity_group_finish_receipts",
        }
    }

    fn trigger_name(self) -> String {
        format!("test_only_scoped_finish_{}_failure", self.slug())
    }

    fn function_name(self) -> String {
        format!("test_only_scoped_finish_{}_failure_fn", self.slug())
    }

    fn marker_name(self) -> String {
        format!("test_only_scoped_finish_{}_failure_marker", self.slug())
    }

    fn canary(self) -> &'static str {
        match self {
            Self::Audit => "secret-scoped-finish-audit-canary-7d9423f1",
            Self::Outbox => "secret-scoped-finish-outbox-canary-41d88dd2",
            Self::Receipt => "secret-scoped-finish-receipt-canary-a3bf9e64",
            Self::DeferredCommit => "secret-scoped-finish-commit-canary-c5ea97b0",
        }
    }

    fn trigger_sql(self) -> String {
        let trigger = self.trigger_name();
        let function = self.function_name();
        match self {
            Self::Audit => format!(
                "CREATE TRIGGER {trigger} BEFORE INSERT ON audit_records \
                 FOR EACH ROW WHEN(NEW.action='capacity.member.finished') \
                 EXECUTE FUNCTION public.{function}()"
            ),
            Self::Outbox => format!(
                "CREATE TRIGGER {trigger} BEFORE INSERT ON event_outbox \
                 FOR EACH ROW WHEN(NEW.event_type='capacity.scoped_member.finished') \
                 EXECUTE FUNCTION public.{function}()"
            ),
            Self::Receipt => format!(
                "CREATE TRIGGER {trigger} BEFORE INSERT ON capacity_group_finish_receipts \
                 FOR EACH ROW WHEN(NEW.member_kind='SCOPED_PATH') \
                 EXECUTE FUNCTION public.{function}()"
            ),
            Self::DeferredCommit => format!(
                "CREATE CONSTRAINT TRIGGER {trigger} AFTER INSERT \
                 ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED \
                 FOR EACH ROW WHEN(NEW.member_kind='SCOPED_PATH') \
                 EXECUTE FUNCTION public.{function}()"
            ),
        }
    }
}

async fn user_trigger_snapshot(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT c.relname||'.'||t.tgname \
         FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid \
         WHERE NOT t.tgisinternal AND c.relname IN \
         ('capacity_groups','capacity_group_members','capacity_group_finish_receipts',\
          'audit_records','event_outbox') \
         ORDER BY c.relname,t.tgname",
    )
    .fetch_all(pool)
    .await
    .expect("schema guard inventory")
}

async fn install_owner_only_failure(pool: &PgPool, stage: FailureStage) {
    let marker = stage.marker_name();
    let function = stage.function_name();
    let ddl = format!(
        "CREATE SEQUENCE public.{marker}; \
         REVOKE ALL ON SEQUENCE public.{marker} FROM PUBLIC; \
         REVOKE ALL ON SEQUENCE public.{marker} FROM sagat_app; \
         CREATE FUNCTION public.{function}() RETURNS trigger \
         LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ \
         BEGIN \
           PERFORM nextval('public.{marker}'::pg_catalog.regclass); \
           RAISE EXCEPTION '{}' USING ERRCODE='23514'; \
         END $$; \
         REVOKE ALL ON FUNCTION public.{function}() FROM PUBLIC; \
         REVOKE ALL ON FUNCTION public.{function}() FROM sagat_app; \
         {};",
        stage.canary(),
        stage.trigger_sql(),
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install labelled owner-only scoped Finish fault");

    let trigger_definition: String = sqlx::query_scalar(
        "SELECT pg_get_triggerdef(t.oid) FROM pg_trigger t \
         JOIN pg_class c ON c.oid=t.tgrelid \
         WHERE c.relname=$1 AND t.tgname=$2 AND NOT t.tgisinternal AND t.tgenabled='O'",
    )
    .bind(stage.relation())
    .bind(stage.trigger_name())
    .fetch_one(pool)
    .await
    .expect("exact enabled fault trigger");
    if matches!(stage, FailureStage::DeferredCommit) {
        assert!(trigger_definition.contains("AFTER INSERT"));
        assert!(trigger_definition.contains("DEFERRABLE INITIALLY DEFERRED"));
    } else {
        assert!(trigger_definition.contains("BEFORE INSERT"));
    }

    let (sequence_usage, function_execute): (bool, bool) = sqlx::query_as(
        "SELECT has_sequence_privilege('sagat_app',$1,'USAGE'), \
         has_function_privilege('sagat_app',$2,'EXECUTE')",
    )
    .bind(format!("public.{marker}"))
    .bind(format!("public.{function}()"))
    .fetch_one(pool)
    .await
    .expect("fault objects remain owner-only");
    assert_eq!((sequence_usage, function_execute), (false, false));
}

async fn assert_stage_was_reached(pool: &PgPool, stage: FailureStage) {
    let marker = stage.marker_name();
    let query = format!("SELECT last_value,is_called FROM public.{marker}");
    let marker_state: (i64, bool) = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .fetch_one(pool)
        .await
        .expect("nontransactional stage marker");
    assert_eq!(
        marker_state,
        (1, true),
        "the exact {:?} trigger ran once before its injected 23514",
        stage
    );
}

async fn remove_owner_only_failure(pool: &PgPool, stage: FailureStage) {
    let ddl = format!(
        "DROP TRIGGER {} ON {}; \
         DROP FUNCTION public.{}(); \
         DROP SEQUENCE public.{};",
        stage.trigger_name(),
        stage.relation(),
        stage.function_name(),
        stage.marker_name(),
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("remove labelled owner-only scoped Finish fault");
}

fn assert_sanitized_confirmed_abort(error: &ScopedLeafFinishError, stage: FailureStage) {
    match error {
        ScopedLeafFinishError::Storage(StoreError::Internal { detail }) => {
            assert_eq!(
                detail,
                "scoped capacity finish storage failure (SQLSTATE 23514)"
            );
            assert!(!detail.contains(stage.canary()));
        }
        other => panic!(
            "the confirmed {:?} 23514 must be Storage, never success or unknown: {other:?}",
            stage
        ),
    }
    let display = format!("{error}");
    let debug = format!("{error:?}");
    assert_eq!(display, "confirmed storage failure");
    assert_eq!(debug, display);
    assert!(!display.contains(stage.canary()));
    assert!(!debug.contains(stage.canary()));
}

#[tokio::test]
async fn native_scoped_finish_statement_and_deferred_commit_failures_are_atomic() {
    for stage in FailureStage::ALL {
        // `certified_flow` creates a fresh disposable database, drives a public flat collection,
        // writes its genuine native certificate, revokes certificate INSERT, adds only the four
        // scoped receipt columns, and returns an explicit sagat_app pool.
        let (f, sources, pool) = certified_flow(
            &format!("scoped-finish-atomic-{}", stage.slug().replace('_', "-")),
            1,
        )
        .await;
        let selected = CapacityMemberId::generate();
        let neighbor = CapacityMemberId::generate();
        let seeded = seed_group(
            &f,
            vec![
                active_member(selected, sources[0].identity.clone()),
                active_member(
                    neighbor,
                    identity_at(&sources[0].identity, "fixture_atomic_neighbor", 9),
                ),
            ],
            7,
            3,
            Some(0),
        )
        .await;
        let command = finish_command(&sources[0], seeded, 7, 3);
        let operation_key = command.operation_key;
        let fingerprint = command.fingerprint().expect("unchanged valid command");
        let source_before = source_execution_snapshot(&f).await;
        let capacity_before = capacity_snapshot(&f, seeded.group).await;
        let nonselected_before = nonselected_members(&f, seeded.group, seeded.selected).await;
        assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));

        let guards_before = user_trigger_snapshot(&f.api.db.pool).await;
        install_owner_only_failure(&f.api.db.pool, stage).await;
        let mut guards_with_fault = guards_before.clone();
        guards_with_fault.push(format!("{}.{}", stage.relation(), stage.trigger_name()));
        guards_with_fault.sort();
        assert_eq!(
            user_trigger_snapshot(&f.api.db.pool).await,
            guards_with_fault,
            "the only schema-trigger change is the named test injector"
        );

        let store = PgScopedLeafFinishStore::new(pool.clone());
        // This deadline is a test watchdog. Expiry panics and establishes no Finish outcome.
        let failure = timeout(ATTEMPT_DEADLINE, store.finish_scoped_leaf(&command))
            .await
            .expect("watchdog expired; timeout is not a transaction outcome")
            .expect_err("the exact owner-installed failure must abort Finish");
        assert_sanitized_confirmed_abort(&failure, stage);
        assert_stage_was_reached(&f.api.db.pool, stage).await;

        assert_eq!(command.operation_key, operation_key);
        assert_eq!(
            command.fingerprint().expect("same retained command"),
            fingerprint
        );
        assert_eq!(source_execution_snapshot(&f).await, source_before);
        assert_eq!(
            capacity_snapshot(&f, seeded.group).await,
            capacity_before,
            "G, selected member, receipt, audit and hint all roll back at {:?}",
            stage
        );
        assert_eq!(
            nonselected_members(&f, seeded.group, seeded.selected).await,
            nonselected_before
        );
        assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));

        remove_owner_only_failure(&f.api.db.pool, stage).await;
        assert_eq!(
            user_trigger_snapshot(&f.api.db.pool).await,
            guards_before,
            "all production schema guards remain exactly installed"
        );

        let applied = timeout(ATTEMPT_DEADLINE, store.finish_scoped_leaf(&command))
            .await
            .expect("watchdog expired; timeout is not a transaction outcome")
            .expect("same retained command applies after removing the injector");
        assert!(!applied.replayed);
        assert_eq!(applied.group_id, seeded.group);
        assert_eq!(applied.member_id, selected);
        assert_eq!(applied.original_run_id, f.run);
        assert_eq!(
            (
                applied.revision,
                applied.epoch,
                applied.active_members,
                applied.possession,
            ),
            (8, 4, 1, CapacityPossession::Vacant)
        );
        assert_header(&f, seeded.group, (8, 4, 1, "VACANT", None)).await;
        assert_eq!(source_execution_snapshot(&f).await, source_before);
        assert_eq!(
            nonselected_members(&f, seeded.group, seeded.selected).await,
            nonselected_before
        );
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        assert_linked_effect(&f, &command, &applied).await;

        let capacity_after_apply = capacity_snapshot(&f, seeded.group).await;
        let replay = timeout(ATTEMPT_DEADLINE, store.finish_scoped_leaf(&command))
            .await
            .expect("watchdog expired; timeout is not a transaction outcome")
            .expect("same retained key replays after one application");
        assert!(replay.replayed);
        let mut expected_replay = applied.clone();
        expected_replay.replayed = true;
        assert_eq!(replay, expected_replay);
        assert_eq!(
            capacity_snapshot(&f, seeded.group).await,
            capacity_after_apply,
            "receipt replay has no second capacity/audit/hint effect"
        );
        assert_eq!(source_execution_snapshot(&f).await, source_before);
        assert_eq!(
            nonselected_members(&f, seeded.group, seeded.selected).await,
            nonselected_before
        );
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        crate::hint_contract::assert_outbox_matches_the_contract(&f.api.db.pool).await;
        f.verify().await;
    }
}
