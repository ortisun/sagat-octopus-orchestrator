//! What the journal of a run becomes as spans and measurements (FR-008-001, US01, US02).
#![allow(clippy::expect_used, clippy::panic)] // helpers fail the test by panicking

use std::collections::HashMap;

use sagat_application::telemetry::{
    AttemptOutcome, CONTEXT_EVENTS, JournalEntry, Observation, RunFacts, attempts, derive,
};
use sagat_domain::execution::{
    FailureCode, Fence, ReviewOutcome, RunEvent, RunId, RunStatus, StopReason,
};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::NodeId;
use sagat_telemetry::{AttrValue, SpanData, SpanStatus, TraceId, attempt_span_id, run_span_id};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

fn node(name: &str) -> NodeId {
    NodeId::parse(name).expect("a node id")
}

fn code(text: &str) -> FailureCode {
    FailureCode::parse(text).expect("a code")
}

struct Journal {
    entries: Vec<JournalEntry>,
    clock: OffsetDateTime,
}

impl Journal {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            clock: OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("time"),
        }
    }

    /// Appends `event` `after` milliseconds later than the previous one.
    fn push(&mut self, after_ms: i64, actor: &str, event: RunEvent) -> &mut Self {
        self.clock += Duration::milliseconds(after_ms);
        let seq = self.entries.len() as u64 + 1;
        self.entries.push(JournalEntry {
            seq,
            event,
            actor: actor.to_owned(),
            occurred_at: self.clock,
        });
        self
    }
}

fn facts(steps: &[&str]) -> RunFacts {
    RunFacts {
        tenant_id: TenantId::new(Uuid::new_v4()),
        namespace_id: NamespaceId::new(Uuid::new_v4()),
        run_id: RunId::generate(),
        workflow: "payment".to_owned(),
        version: 3,
        trace_id: TraceId::generate(),
        parent_span: None,
        parent_run: None,
        created_at: OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("time"),
        steps: steps
            .iter()
            .map(|s| ((*s).to_owned(), Uuid::new_v4()))
            .collect(),
    }
}

/// `charge` is leased and fails twice, then succeeds: three attempts.
fn three_attempts() -> Journal {
    let mut j = Journal::new();
    let charge = node("charge");
    j.push(
        0,
        "system:engine",
        RunEvent::RunStarted {
            entrypoint: charge.clone(),
        },
    );
    j.push(
        5,
        "system:engine",
        RunEvent::StepReleased {
            node: charge.clone(),
        },
    );
    for attempt in 1..=3_u32 {
        j.push(
            20,
            "worker:w1",
            RunEvent::StepLeased {
                node: charge.clone(),
                attempt,
                generation: u64::from(attempt),
            },
        );
        if attempt < 3 {
            j.push(
                100,
                "worker:w1",
                RunEvent::StepRetryScheduled {
                    node: charge.clone(),
                    attempt,
                    generation: u64::from(attempt),
                    code: code("PAYMENT_DECLINED"),
                    not_before: OffsetDateTime::from_unix_timestamp(1_800_000_010).expect("time"),
                },
            );
            j.push(
                300,
                "system:scheduler",
                RunEvent::StepRetryDue {
                    node: charge.clone(),
                },
            );
        } else {
            j.push(
                150,
                "worker:w1",
                RunEvent::StepCompleted {
                    node: charge.clone(),
                    attempt,
                    generation: 3,
                },
            );
        }
    }
    j.push(
        1,
        "system:engine",
        RunEvent::RunFinished {
            status: RunStatus::Succeeded,
        },
    );
    j
}

fn named<'a>(spans: &'a [SpanData], name: &str) -> Vec<&'a SpanData> {
    spans.iter().filter(|s| s.name == name).collect()
}

fn attr<'a>(span: &'a SpanData, key: &str) -> Option<&'a AttrValue> {
    span.attributes
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

#[test]
fn a_run_with_three_attempts_has_three_attempt_spans_and_a_run_span() {
    let facts = facts(&["charge"]);
    let journal = three_attempts();
    let derived = derive(&facts, &journal.entries, 0);

    let attempts_spans = named(&derived.spans, "step.attempt");
    assert_eq!(attempts_spans.len(), 3);
    let step = facts.steps["charge"];
    for (n, span) in attempts_spans.iter().enumerate() {
        let attempt = u32::try_from(n + 1).expect("small");
        assert_eq!(
            span.span_id,
            attempt_span_id(step, attempt),
            "the span the lease carried"
        );
        assert_eq!(span.parent, Some(run_span_id(facts.run_id.as_uuid())));
        assert_eq!(span.trace_id, facts.trace_id);
        assert_eq!(
            attr(span, "sagat.attempt"),
            Some(&AttrValue::Int(i64::from(attempt)))
        );
    }
    assert_eq!(
        attr(attempts_spans[0], "sagat.outcome"),
        Some(&AttrValue::from("retry"))
    );
    assert_eq!(
        attr(attempts_spans[0], "sagat.failure.code"),
        Some(&AttrValue::from("PAYMENT_DECLINED"))
    );
    assert_eq!(
        attr(attempts_spans[2], "sagat.outcome"),
        Some(&AttrValue::from("succeeded"))
    );
    assert_eq!(
        attempts_spans[0].status,
        SpanStatus::Error("PAYMENT_DECLINED".to_owned())
    );
    assert_eq!(attempts_spans[2].status, SpanStatus::Ok);
    assert_eq!(attempts_spans[0].events[0].name, "retry scheduled");

    let run = named(&derived.spans, "workflow.run");
    assert_eq!(run.len(), 1);
    assert_eq!(run[0].span_id, run_span_id(facts.run_id.as_uuid()));
    assert_eq!(
        attr(run[0], "sagat.run.status"),
        Some(&AttrValue::from("SUCCEEDED"))
    );
    assert_eq!(
        attr(run[0], "sagat.workflow"),
        Some(&AttrValue::from("payment"))
    );
    assert_eq!(run[0].status, SpanStatus::Ok);
}

#[test]
fn the_duration_of_an_attempt_is_from_its_lease_to_its_end() {
    let facts = facts(&["charge"]);
    let journal = three_attempts();
    let derived = derive(&facts, &journal.entries, 0);
    let first = named(&derived.spans, "step.attempt")[0];
    assert_eq!(first.end_unix_nano - first.start_unix_nano, 100 * 1_000_000);
    let measured: Vec<f64> = derived
        .observations
        .iter()
        .filter_map(|o| match o {
            Observation::AttemptEnded { seconds, .. } => Some(*seconds),
            Observation::RunFinished { .. } => None,
        })
        .collect();
    assert_eq!(measured.len(), 3);
    assert!(
        (measured[0] - 0.1).abs() < 1e-9 && (measured[2] - 0.15).abs() < 1e-9,
        "{measured:?}"
    );
}

#[test]
fn the_wait_between_ready_and_leased_is_the_schedule_latency_of_each_attempt() {
    let facts = facts(&["charge"]);
    let journal = three_attempts();
    let latencies: Vec<Option<f64>> = derive(&facts, &journal.entries, 0)
        .observations
        .iter()
        .filter_map(|o| match o {
            Observation::AttemptEnded {
                schedule_latency, ..
            } => Some(*schedule_latency),
            Observation::RunFinished { .. } => None,
        })
        .collect();
    // The first: released 5 ms after the start, leased 20 ms later. The others: from retry_due.
    assert_eq!(latencies.len(), 3);
    for latency in latencies {
        assert!((latency.expect("known") - 0.02).abs() < 1e-9, "{latency:?}");
    }
}

#[test]
fn deriving_in_two_batches_gives_the_same_spans_as_deriving_at_once_and_none_twice() {
    let facts = facts(&["charge"]);
    let journal = three_attempts();
    let all = derive(&facts, &journal.entries, 0);
    let ids = |d: &sagat_application::telemetry::Derived| {
        let mut ids: Vec<_> = d
            .spans
            .iter()
            .map(|s| (s.name.clone(), s.span_id))
            .collect();
        ids.sort_by_key(|(name, id)| (name.clone(), id.to_string()));
        ids
    };
    for split in 0..=journal.entries.len() {
        let first = derive(&facts, &journal.entries[..split], 0);
        // The second batch sees the whole journal (a stand-in for the context the store loads) and
        // knows what was applied.
        let second = derive(&facts, &journal.entries, split as u64);
        let mut together = ids(&first);
        together.extend(ids(&second));
        together.sort_by_key(|(name, id)| (name.clone(), id.to_string()));
        assert_eq!(together, ids(&all), "split after {split} entries");
        let mut seen = std::collections::HashSet::new();
        assert!(
            together.iter().all(|key| seen.insert(key.clone())),
            "a span twice, split after {split}"
        );
    }
}

#[test]
fn an_attempt_that_has_not_ended_is_not_exported_and_is_when_it_does() {
    let facts = facts(&["charge"]);
    let mut j = Journal::new();
    let charge = node("charge");
    j.push(
        0,
        "system:engine",
        RunEvent::StepReleased {
            node: charge.clone(),
        },
    );
    j.push(
        10,
        "worker:w1",
        RunEvent::StepLeased {
            node: charge.clone(),
            attempt: 1,
            generation: 1,
        },
    );
    assert!(derive(&facts, &j.entries, 0).spans.is_empty());
    let open = attempts(&facts, &j.entries);
    assert_eq!((open.len(), open[0].outcome), (1, AttemptOutcome::Open));

    j.push(
        40,
        "worker:w1",
        RunEvent::StepCompleted {
            node: charge,
            attempt: 1,
            generation: 1,
        },
    );
    let derived = derive(&facts, &j.entries, 2);
    assert_eq!(
        named(&derived.spans, "step.attempt").len(),
        1,
        "the lease was in an earlier batch"
    );
}

#[test]
fn a_lost_lease_is_an_error_attempt_with_its_code() {
    let facts = facts(&["charge"]);
    let mut j = Journal::new();
    let charge = node("charge");
    j.push(
        0,
        "system:engine",
        RunEvent::StepReleased {
            node: charge.clone(),
        },
    );
    j.push(
        10,
        "worker:w1",
        RunEvent::StepLeased {
            node: charge.clone(),
            attempt: 1,
            generation: 1,
        },
    );
    j.push(
        30_000,
        "system:scheduler",
        RunEvent::StepFailed {
            node: charge,
            code: code("LEASE_EXPIRED"),
            fence: Some(Fence {
                attempt: 1,
                generation: 1,
            }),
        },
    );
    let derived = derive(&facts, &j.entries, 0);
    let span = named(&derived.spans, "step.attempt")[0];
    assert_eq!(span.status, SpanStatus::Error("LEASE_EXPIRED".to_owned()));
    assert_eq!(
        attr(span, "sagat.outcome"),
        Some(&AttrValue::from("failed"))
    );
}

#[test]
fn a_step_cancelled_while_it_held_the_lease_ends_its_attempt_as_cancelled() {
    let facts = facts(&["charge"]);
    let mut j = Journal::new();
    let charge = node("charge");
    j.push(
        0,
        "system:engine",
        RunEvent::StepReleased {
            node: charge.clone(),
        },
    );
    j.push(
        10,
        "worker:w1",
        RunEvent::StepLeased {
            node: charge.clone(),
            attempt: 1,
            generation: 1,
        },
    );
    j.push(
        10,
        "alice",
        RunEvent::StepCancelRequested {
            node: charge.clone(),
        },
    );
    j.push(
        10,
        "system:engine",
        RunEvent::StepCancelled { node: charge },
    );
    let derived = derive(&facts, &j.entries, 0);
    let span = named(&derived.spans, "step.attempt")[0];
    assert_eq!(
        attr(span, "sagat.outcome"),
        Some(&AttrValue::from("cancelled"))
    );
    assert_eq!(span.status, SpanStatus::Unset, "a stop is not an error");
}

#[test]
fn a_step_cancelled_between_attempts_does_not_close_an_attempt_that_already_ended() {
    let facts = facts(&["charge"]);
    let mut j = Journal::new();
    let charge = node("charge");
    j.push(
        0,
        "system:engine",
        RunEvent::StepReleased {
            node: charge.clone(),
        },
    );
    j.push(
        10,
        "worker:w1",
        RunEvent::StepLeased {
            node: charge.clone(),
            attempt: 1,
            generation: 1,
        },
    );
    j.push(
        10,
        "worker:w1",
        RunEvent::StepRetryScheduled {
            node: charge.clone(),
            attempt: 1,
            generation: 1,
            code: code("TIMEOUT"),
            not_before: OffsetDateTime::UNIX_EPOCH,
        },
    );
    j.push(
        10,
        "system:engine",
        RunEvent::StepCancelled { node: charge },
    );
    let derived = derive(&facts, &j.entries, 0);
    let spans = named(&derived.spans, "step.attempt");
    assert_eq!(spans.len(), 1);
    assert_eq!(
        attr(spans[0], "sagat.outcome"),
        Some(&AttrValue::from("retry"))
    );
}

#[test]
fn timers_and_waits_are_spans_of_the_time_a_run_spent_not_working() {
    let facts = facts(&["pause", "approval"]);
    let mut j = Journal::new();
    let (pause, approval) = (node("pause"), node("approval"));
    j.push(
        0,
        "system:engine",
        RunEvent::TimerStarted {
            node: pause.clone(),
            due_at: OffsetDateTime::UNIX_EPOCH,
        },
    );
    j.push(
        2_000,
        "system:scheduler",
        RunEvent::TimerFired { node: pause },
    );
    j.push(
        0,
        "system:engine",
        RunEvent::WaitStarted {
            node: approval.clone(),
            correlation_key: sagat_domain::execution::CorrelationKey::parse("order-7")
                .expect("key"),
            deadline: OffsetDateTime::UNIX_EPOCH,
        },
    );
    j.push(
        500,
        "system:scheduler",
        RunEvent::WaitTimedOut {
            node: approval,
            handled: false,
        },
    );
    let derived = derive(&facts, &j.entries, 0);
    let timers = named(&derived.spans, "timer");
    assert_eq!(timers.len(), 1);
    assert_eq!(
        timers[0].end_unix_nano - timers[0].start_unix_nano,
        2_000 * 1_000_000
    );
    let waits = named(&derived.spans, "wait.event");
    assert_eq!(waits.len(), 1);
    assert_eq!(
        waits[0].status,
        SpanStatus::Error("WAIT_TIMED_OUT".to_owned())
    );
    assert_ne!(timers[0].span_id, waits[0].span_id);
}

#[test]
fn the_run_span_hangs_from_the_request_the_parent_run_or_nothing() {
    let mut j = Journal::new();
    j.push(
        0,
        "system:engine",
        RunEvent::RunFinished {
            status: RunStatus::Succeeded,
        },
    );

    let mut with_parent_span = facts(&[]);
    with_parent_span.parent_span = Some(sagat_telemetry::SpanId::generate());
    let span = &derive(&with_parent_span, &j.entries, 0).spans[0];
    assert_eq!(span.parent, with_parent_span.parent_span);

    let mut child = facts(&[]);
    let parent_run = RunId::generate();
    child.parent_run = Some(parent_run);
    child.parent_span = Some(sagat_telemetry::SpanId::generate());
    let span = &derive(&child, &j.entries, 0).spans[0];
    assert_eq!(
        span.parent,
        Some(run_span_id(parent_run.as_uuid())),
        "a child hangs from its parent's run"
    );

    assert_eq!(derive(&facts(&[]), &j.entries, 0).spans[0].parent, None);
}

#[test]
fn a_failed_run_is_an_error_span_and_a_cancelled_one_is_not() {
    for (status, error) in [
        (RunStatus::Failed, true),
        (RunStatus::TimedOut, true),
        (RunStatus::CompensationFailed, true),
        (RunStatus::Partial, true),
        (RunStatus::Cancelled, false),
        (RunStatus::Compensated, false),
    ] {
        let mut j = Journal::new();
        j.push(0, "system:engine", RunEvent::RunFinished { status });
        let span = &derive(&facts(&[]), &j.entries, 0).spans[0];
        assert_eq!(
            matches!(span.status, SpanStatus::Error(_)),
            error,
            "{}",
            status.as_str()
        );
    }
}

#[test]
fn what_an_operator_did_is_on_the_run_span_without_saying_who() {
    let facts = facts(&["charge"]);
    let mut j = Journal::new();
    j.push(
        0,
        "system:engine",
        RunEvent::RunStarted {
            entrypoint: node("charge"),
        },
    );
    j.push(5, "alice@example.com", RunEvent::RunPaused);
    j.push(5, "alice@example.com", RunEvent::RunResumed);
    j.push(
        5,
        "bob@example.com",
        RunEvent::StepResolved {
            node: node("charge"),
            outcome: ReviewOutcome::Succeeded,
        },
    );
    j.push(
        5,
        "alice@example.com",
        RunEvent::RunStopRequested {
            reason: StopReason::Cancelled,
            code: None,
        },
    );
    j.push(
        5,
        "system:engine",
        RunEvent::RunFinished {
            status: RunStatus::Cancelled,
        },
    );
    let derived = derive(&facts, &j.entries, 0);
    let run = named(&derived.spans, "workflow.run")[0];
    let names: Vec<&str> = run.events.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "run.paused",
            "run.resumed",
            "step.resolved",
            "run.stop_requested"
        ]
    );
    let raw = format!("{:?}", derived.spans);
    assert!(
        !raw.contains("alice") && !raw.contains("bob"),
        "a person's subject is in the spans: {raw}"
    );
    assert!(
        run.events[0]
            .attributes
            .iter()
            .any(|(k, v)| k == "sagat.actor.kind" && *v == AttrValue::from("user"))
    );
}

#[test]
fn a_run_finished_is_counted_with_its_duration() {
    let facts = facts(&["charge"]);
    let journal = three_attempts();
    let finished: Vec<(&str, f64)> = derive(&facts, &journal.entries, 0)
        .observations
        .iter()
        .filter_map(|o| match o {
            Observation::RunFinished { status, seconds } => Some((*status, *seconds)),
            Observation::AttemptEnded { .. } => None,
        })
        .collect();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].0, "SUCCEEDED");
    assert!(finished[0].1 > 0.5, "{finished:?}");
}

#[test]
fn the_attempts_of_two_steps_do_not_get_each_others_ends() {
    let facts = facts(&["a", "b"]);
    let mut j = Journal::new();
    let (a, b) = (node("a"), node("b"));
    j.push(
        0,
        "system:engine",
        RunEvent::StepReleased { node: a.clone() },
    );
    j.push(
        0,
        "system:engine",
        RunEvent::StepReleased { node: b.clone() },
    );
    j.push(
        1,
        "worker:w1",
        RunEvent::StepLeased {
            node: a.clone(),
            attempt: 1,
            generation: 1,
        },
    );
    j.push(
        1,
        "worker:w2",
        RunEvent::StepLeased {
            node: b.clone(),
            attempt: 1,
            generation: 1,
        },
    );
    j.push(
        5,
        "worker:w2",
        RunEvent::StepCompleted {
            node: b,
            attempt: 1,
            generation: 1,
        },
    );
    j.push(
        5,
        "worker:w1",
        RunEvent::StepCompleted {
            node: a,
            attempt: 1,
            generation: 1,
        },
    );
    let by_node: HashMap<String, u64> = derive(&facts, &j.entries, 0)
        .spans
        .iter()
        .filter(|s| s.name == "step.attempt")
        .map(|s| {
            let node = match attr(s, "sagat.node") {
                Some(AttrValue::Str(n)) => n.clone(),
                other => panic!("{other:?}"),
            };
            (node, s.end_unix_nano - s.start_unix_nano)
        })
        .collect();
    // `a` was leased 1 ms before `b`, and ended 5 ms after it: 1 + 5 + 5.
    assert_eq!(by_node["a"], 11 * 1_000_000);
    assert_eq!(by_node["b"], 5 * 1_000_000);
}

#[test]
fn a_timer_that_starts_in_one_batch_and_fires_in_a_later_one_gets_its_span() {
    // The store gives a batch the new entries and, of the part already projected, only the events
    // `CONTEXT_EVENTS` names. A timer that began before the split and fires after it must not be
    // lost because its start was left out.
    let facts = facts(&["charge"]);
    let mut j = Journal::new();
    let sleep = node("sleep");
    j.push(
        0,
        "system:engine",
        RunEvent::RunStarted {
            entrypoint: sleep.clone(),
        },
    );
    j.push(
        5,
        "system:engine",
        RunEvent::TimerStarted {
            node: sleep.clone(),
            due_at: OffsetDateTime::from_unix_timestamp(1_800_003_600).expect("time"),
        },
    );
    j.push(
        3_600_000,
        "system:scheduler",
        RunEvent::TimerFired {
            node: sleep.clone(),
        },
    );
    // The first batch took the first two entries; the second sees the third and the filtered rest.
    let applied = 2;
    let kept: Vec<JournalEntry> = j
        .entries
        .iter()
        .filter(|e| e.seq <= applied && CONTEXT_EVENTS.contains(&e.event.name()))
        .chain(j.entries.iter().filter(|e| e.seq > applied))
        .cloned()
        .collect();
    let second = derive(&facts, &kept, applied);
    assert!(
        second.spans.iter().any(|s| s.name == "timer"),
        "the span of the timer: {:?}",
        second
            .spans
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_wait_that_starts_in_one_batch_and_is_satisfied_in_a_later_one_gets_its_span() {
    let facts = facts(&["charge"]);
    let mut j = Journal::new();
    let hold = node("hold");
    j.push(
        0,
        "system:engine",
        RunEvent::RunStarted {
            entrypoint: hold.clone(),
        },
    );
    j.push(
        5,
        "system:engine",
        RunEvent::WaitStarted {
            node: hold.clone(),
            correlation_key: sagat_domain::execution::CorrelationKey::parse("order-1")
                .expect("key"),
            deadline: OffsetDateTime::from_unix_timestamp(1_800_003_600).expect("time"),
        },
    );
    j.push(
        60_000,
        "system:events",
        RunEvent::WaitSatisfied { node: hold.clone() },
    );
    let applied = 2;
    let kept: Vec<JournalEntry> = j
        .entries
        .iter()
        .filter(|e| e.seq <= applied && CONTEXT_EVENTS.contains(&e.event.name()))
        .chain(j.entries.iter().filter(|e| e.seq > applied))
        .cloned()
        .collect();
    let second = derive(&facts, &kept, applied);
    assert!(
        second.spans.iter().any(|s| s.name == "wait.event"),
        "the span of the wait: {:?}",
        second
            .spans
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    );
}
