//! Schedules: create, describe, list, update, delete, pause, resume, trigger, backfill and the
//! history of their decisions, reached from [`Client::schedules`]. Every call that changes
//! something carries one `Idempotency-Key` (given, or generated once) that every retry keeps.
use super::wire::{
    Fields, Moment, body_of, id_segment, namespace_path, positive_revision, push_page, request,
    summary_debug, wire_enum, with,
};
use super::*;
use crate::Page;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

wire_enum! {
    /// What a schedule does when its previous action is still running.
    OverlapPolicy {
        Skip => "SKIP",
        BufferOne => "BUFFER_ONE",
        BufferAll => "BUFFER_ALL",
        CancelOther => "CANCEL_OTHER",
        AllowAll => "ALLOW_ALL",
    }
}
wire_enum! {
    /// How many missed times are acted on after an outage.
    CatchupMode { One => "ONE", All => "ALL" }
}
wire_enum! {
    /// Which version of the workflow a schedule starts.
    VersionPolicy { Active => "ACTIVE", Pinned => "PINNED" }
}

/// A range of a calendar field: `start`, optionally to `end`, optionally every `step`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRange {
    pub start: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
}
impl ScheduleRange {
    pub fn at(start: u32) -> Self {
        Self {
            start,
            ..Self::default()
        }
    }
}

/// A calendar of nominal times; a field that is empty is left out of the request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCalendar {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub second: Vec<ScheduleRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub minute: Vec<ScheduleRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hour: Vec<ScheduleRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub day_of_month: Vec<ScheduleRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub month: Vec<ScheduleRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub year: Vec<ScheduleRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub day_of_week: Vec<ScheduleRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleInterval {
    pub every_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_ms: Option<u64>,
}

/// What happens to a time that falls in a gap or twice in an ambiguous hour of the zone
/// (`gap`: `SKIP` or `SHIFT_FORWARD`; `ambiguous`: `ONCE_EARLIER` or `TWICE`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DstPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambiguous: Option<String>,
}

/// When a schedule acts. `start_at` and `end_at` are sent in UTC.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<ScheduleCalendar>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cron: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intervals: Vec<ScheduleInterval>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<ScheduleCalendar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<Moment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_at: Option<Moment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<DstPolicy>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePolicies {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlap: Option<OverlapPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catchup_window_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catchup_mode: Option<CatchupMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_on_failure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer_limit: Option<u32>,
}

/// What a schedule starts at each of its times.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleAction {
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_policy: Option<VersionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    /// The input of every run, at most 64 KiB. It is never returned: only its digest is.
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal_time_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_timeout_ms: Option<u64>,
    /// Every run the schedule starts gets the business id `<prefix>-<nominal time>`. Customer text,
    /// sent as given: no trimming, no case folding and no local pattern or length check (the
    /// server accepts 1 to 160 visible ASCII characters and refuses the rest with
    /// `VALIDATION_FAILED`, reason `SCHEDULE_SPEC_INVALID`). It is never part of the `Debug` of the
    /// action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id_prefix: Option<String>,
    /// What an action does when an open execution holds its id; only with a prefix (`FAIL` when
    /// absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id_conflict_policy: Option<BusinessIdConflictPolicy>,
    /// What an action does when a closed execution still remembers its id; only with a prefix
    /// (`ALLOW_DUPLICATE` when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id_reuse_policy: Option<BusinessIdReusePolicy>,
}
impl std::fmt::Debug for ScheduleAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduleAction")
            .field("workflow_id", &self.workflow_id)
            .finish_non_exhaustive()
    }
}
impl ScheduleAction {
    pub fn new(workflow_id: impl Into<String>, input: Value) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            version_policy: None,
            version_id: None,
            input,
            nominal_time_field: None,
            run_timeout_ms: None,
            business_id_prefix: None,
            business_id_conflict_policy: None,
            business_id_reuse_policy: None,
        }
    }
}

/// What a schedule is written as (`create`, `update`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleWrite {
    pub name: String,
    pub spec: ScheduleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policies: Option<SchedulePolicies>,
    pub action: ScheduleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_actions: Option<u64>,
}
impl ScheduleWrite {
    pub fn new(name: impl Into<String>, spec: ScheduleSpec, action: ScheduleAction) -> Self {
        Self {
            name: name.into(),
            spec,
            policies: None,
            action,
            paused: None,
            note: None,
            remaining_actions: None,
        }
    }
    /// The request body: the instants of the spec are written in UTC, and text that is not an
    /// instant is refused before anything is sent.
    fn body(&self) -> Result<Value, SagatError> {
        if let Some(at) = &self.spec.start_at {
            at.utc_text("startAt")?;
        }
        if let Some(at) = &self.spec.end_at {
            at.utc_text("endAt")?;
        }
        body_of(self)
    }
}

/// The action as the server keeps it: without the input, with its digest.
#[derive(Clone, PartialEq, Eq)]
pub struct ScheduleActionDefinition {
    pub workflow_id: Option<String>,
    pub version_policy: Option<String>,
    pub version_id: Option<String>,
    pub input_digest: Option<String>,
    pub nominal_time_field: Option<String>,
    pub run_timeout_ms: Option<u64>,
    /// The business-id prefix of every run the schedule starts, as the server sent it; `None` when
    /// the action has none. Customer text: it is never part of the `Debug` of the action.
    pub business_id_prefix: Option<String>,
    /// `FAIL` or `USE_EXISTING` (or a value of a newer server, kept as sent); present with a prefix.
    pub business_id_conflict_policy: Option<String>,
    /// `ALLOW_DUPLICATE`, `ALLOW_DUPLICATE_FAILED_ONLY` or `REJECT_DUPLICATE` (or a value of a
    /// newer server, kept as sent); present with a prefix.
    pub business_id_reuse_policy: Option<String>,
}
impl std::fmt::Debug for ScheduleActionDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduleActionDefinition")
            .field("workflow_id", &self.workflow_id)
            .field("version_policy", &self.version_policy)
            .field("version_id", &self.version_id)
            .field("input_digest", &self.input_digest)
            .field("nominal_time_field", &self.nominal_time_field)
            .field("run_timeout_ms", &self.run_timeout_ms)
            .field(
                "business_id_conflict_policy",
                &self.business_id_conflict_policy,
            )
            .field("business_id_reuse_policy", &self.business_id_reuse_policy)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct Schedule {
    pub schedule_id: Uuid,
    pub name: String,
    pub origin: String,
    pub state: String,
    pub revision: u64,
    pub exhausted: bool,
    pub action: ScheduleActionDefinition,
    pub note: Option<String>,
    pub note_by: Option<String>,
    pub remaining_actions: Option<u64>,
    pub next_action_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: Option<OffsetDateTime>,
    /// The answer as the server sent it, so a field of a newer server is reachable.
    pub raw: Value,
}
summary_debug!(Schedule {
    schedule_id,
    state,
    revision
});
impl Schedule {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        let f = Fields::of(&raw, "a schedule")?;
        Self::from_fields(&f, raw.clone())
    }
    fn from_fields(f: &Fields<'_>, raw: Value) -> Result<Self, SagatError> {
        let action = f.object("action");
        let part = |key: &str| action.as_ref().and_then(|a| a.opt_text(key));
        Ok(Self {
            schedule_id: f.uuid("scheduleId")?,
            name: f.text("name")?,
            origin: f.text("origin")?,
            state: f.text("state")?,
            revision: f.at_least("revision", 1)?,
            exhausted: f.flag("exhausted"),
            action: ScheduleActionDefinition {
                workflow_id: part("workflowId"),
                version_policy: part("versionPolicy"),
                version_id: part("versionId"),
                input_digest: part("inputDigest"),
                nominal_time_field: part("nominalTimeField"),
                run_timeout_ms: action.as_ref().and_then(|a| a.opt_u64("runTimeoutMs")),
                business_id_prefix: part("businessIdPrefix"),
                business_id_conflict_policy: part("businessIdConflictPolicy"),
                business_id_reuse_policy: part("businessIdReusePolicy"),
            },
            note: f.opt_text("note"),
            note_by: f.opt_text("noteBy"),
            remaining_actions: f.opt_u64("remainingActions"),
            next_action_at: f.opt_instant("nextActionAt")?,
            created_at: f.instant("createdAt")?,
            updated_at: f.opt_instant("updatedAt")?,
            raw,
        })
    }
}

/// A decision of a schedule about one nominal time.
#[derive(Clone)]
pub struct ScheduleActionRecord {
    pub nominal_time: OffsetDateTime,
    pub actual_time: Option<OffsetDateTime>,
    pub kind: String,
    pub origin: Option<String>,
    pub revision: u64,
    /// One of the outcomes this SDK knows, or another string: the set only grows.
    pub outcome: String,
    pub reason: Option<String>,
    pub run_id: Option<Uuid>,
    pub backfill_id: Option<Uuid>,
    pub summary_count: Option<u64>,
    pub summary_last_nominal_time: Option<OffsetDateTime>,
    pub decided_at: OffsetDateTime,
    pub raw: Value,
}
summary_debug!(ScheduleActionRecord {
    kind,
    outcome,
    reason
});
impl ScheduleActionRecord {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        Self::read(&raw)
    }
    fn read(raw: &Value) -> Result<Self, SagatError> {
        let f = Fields::of(raw, "a schedule action")?;
        Ok(Self {
            nominal_time: f.instant("nominalTime")?,
            actual_time: f.opt_instant("actualTime")?,
            kind: f.text("kind")?,
            origin: f.opt_text("origin"),
            revision: f.at_least("revision", 1)?,
            outcome: f.text("outcome")?,
            reason: f.opt_text("reason"),
            run_id: f.opt_uuid("runId")?,
            backfill_id: f.opt_uuid("backfillId")?,
            summary_count: f.opt_u64("summaryCount"),
            summary_last_nominal_time: f.opt_instant("summaryLastNominalTime")?,
            decided_at: f.instant("decidedAt")?,
            raw: raw.clone(),
        })
    }
}

/// A time a schedule will act at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpcomingAction {
    pub nominal_time: OffsetDateTime,
    pub actual_time: OffsetDateTime,
}

/// A schedule with its next times, its latest decisions, the runs it has open and its counters.
/// It reads as the [`Schedule`] it describes.
#[derive(Clone)]
pub struct ScheduleDescription {
    pub schedule: Schedule,
    pub next_actions: Vec<UpcomingAction>,
    pub recent_actions: Vec<ScheduleActionRecord>,
    pub open_runs: Vec<Uuid>,
    pub buffered_count: u64,
    pub counters: BTreeMap<String, u64>,
}
summary_debug!(ScheduleDescription { schedule });
impl std::ops::Deref for ScheduleDescription {
    type Target = Schedule;
    fn deref(&self) -> &Schedule {
        &self.schedule
    }
}
impl ScheduleDescription {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        let f = Fields::of(&raw, "a schedule description")?;
        let next_actions = f
            .values("nextActions")
            .iter()
            .map(|entry| {
                let e = Fields::of(entry, "a next action")?;
                Ok(UpcomingAction {
                    nominal_time: e.instant("nominalTime")?,
                    actual_time: e.instant("actualTime")?,
                })
            })
            .collect::<Result<Vec<_>, SagatError>>()?;
        let recent_actions = f
            .values("recentActions")
            .iter()
            .map(ScheduleActionRecord::read)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            schedule: Schedule::from_fields(&f, raw.clone())?,
            next_actions,
            recent_actions,
            open_runs: f.uuids("openRuns")?,
            buffered_count: f.count("bufferedCount"),
            counters: f.counters("counters"),
        })
    }
}

/// A range of past nominal times a schedule was asked to act on.
#[derive(Clone)]
pub struct ScheduleBackfill {
    pub backfill_id: Uuid,
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
    pub cursor: Option<OffsetDateTime>,
    pub state: String,
    pub overlap: String,
    pub counts: BTreeMap<String, u64>,
    pub raw: Value,
}
summary_debug!(ScheduleBackfill { backfill_id, state });
impl ScheduleBackfill {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        Self::read(&raw)
    }
    fn read(raw: &Value) -> Result<Self, SagatError> {
        let f = Fields::of(raw, "a backfill")?;
        Ok(Self {
            backfill_id: f.uuid("backfillId")?,
            start: f.instant("start")?,
            end: f.instant("end")?,
            cursor: f.opt_instant("cursor")?,
            state: f.text("state")?,
            overlap: f.text("overlap")?,
            counts: f.counters("counts"),
            raw: raw.clone(),
        })
    }
}

/// How many upcoming and recent entries `get` describes; the server's defaults when absent.
#[derive(Clone, Debug, Default)]
pub struct ScheduleDescribeOptions {
    pub next_count: Option<u32>,
    pub recent_count: Option<u32>,
}
/// The filters of `list`; `name` selects names that start with that text.
#[derive(Clone, Debug, Default)]
pub struct ScheduleListFilter {
    /// `ACTIVE` or `PAUSED`.
    pub state: Option<String>,
    /// `API` or `TRIGGER`.
    pub origin: Option<String>,
    pub workflow_id: Option<String>,
    pub name: Option<String>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct ScheduleActionFilter {
    pub outcome: Option<String>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}

/// The schedules of a namespace, reached from [`Client::schedules`].
#[derive(Clone, Copy)]
pub struct Schedules<'a> {
    client: &'a Client,
}
impl std::fmt::Debug for Schedules<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Schedules").finish_non_exhaustive()
    }
}
impl Client {
    /// Schedules: create, describe, list, update, delete, pause, resume, trigger, backfill and
    /// their history.
    pub fn schedules(&self) -> Schedules<'_> {
        Schedules { client: self }
    }
}

fn path(
    namespace_id: &str,
    schedule_id: Option<&str>,
    tail: &[&str],
) -> Result<Vec<String>, SagatError> {
    let mut path = with(&namespace_path(namespace_id)?, &["schedules"]);
    if let Some(id) = schedule_id {
        path.push(id_segment(id, "a schedule id")?);
    }
    path.extend(tail.iter().map(|s| (*s).to_owned()));
    Ok(path)
}
fn note_of(note: &str) -> Result<Value, SagatError> {
    if note.is_empty() {
        return Err(SagatError::validation("a note is a non-empty text"));
    }
    Ok(json!({"note":note}))
}
fn count_query(name: &str, value: Option<u32>, query: &mut Vec<(String, String)>) {
    if let Some(value) = value {
        query.push((name.into(), value.to_string()));
    }
}

impl Schedules<'_> {
    /// Creates a schedule. A name in use is `CONFLICT`; the same key and body answers what the
    /// first call did.
    pub async fn create(
        &self,
        namespace_id: &str,
        write: &ScheduleWrite,
        call: CallOptions,
    ) -> Result<Schedule, SagatError> {
        let body = write.body()?;
        let req = request(
            Method::POST,
            &path(namespace_id, None, &[])?,
            201,
            Vec::new(),
        )
        .json(&body)?
        .keyed(key(&call)?);
        self.client.execute(req, call).await?.parse(Schedule::parse)
    }

    /// A schedule with its next times, its latest decisions, the runs it has open and its counters.
    pub async fn get(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        options: ScheduleDescribeOptions,
        call: CallOptions,
    ) -> Result<ScheduleDescription, SagatError> {
        let mut query = Vec::new();
        count_query("nextCount", options.next_count, &mut query);
        count_query("recentCount", options.recent_count, &mut query);
        let req = request(
            Method::GET,
            &path(namespace_id, Some(schedule_id), &[])?,
            200,
            query,
        );
        self.client
            .execute(req, call)
            .await?
            .parse(ScheduleDescription::parse)
    }

    /// One page of the schedules the filter selects.
    pub async fn list(
        &self,
        namespace_id: &str,
        filter: ScheduleListFilter,
        call: CallOptions,
    ) -> Result<Page<Schedule>, SagatError> {
        let mut query = Vec::new();
        push_page(&mut query, filter.limit, filter.cursor.as_deref())?;
        for (name, value) in [
            ("state", &filter.state),
            ("origin", &filter.origin),
            ("name", &filter.name),
        ] {
            if let Some(value) = value {
                query.push((name.into(), value.clone()));
            }
        }
        if let Some(workflow) = &filter.workflow_id {
            query.push(("workflowId".into(), id_segment(workflow, "a workflow id")?));
        }
        let req = request(Method::GET, &path(namespace_id, None, &[])?, 200, query);
        self.client
            .execute(req, call)
            .await?
            .parse(|raw| wire::page_of(raw, Schedule::read))
    }

    /// Every schedule the filter selects, following the opaque cursors: the next page is fetched
    /// only as the caller consumes the previous one.
    pub fn list_all(
        &self,
        namespace_id: &str,
        filter: ScheduleListFilter,
        call: CallOptions,
    ) -> impl futures_util::Stream<Item = Result<Schedule, SagatError>> + Send + 'static {
        futures_util::stream::try_unfold(
            (
                self.client.clone(),
                namespace_id.to_owned(),
                filter,
                call,
                VecDeque::<Schedule>::new(),
                false,
            ),
            |(client, namespace_id, mut filter, call, mut buffer, mut finished)| async move {
                loop {
                    if call.cancel.is_cancelled() {
                        return Err(cancelled());
                    }
                    if let Some(item) = buffer.pop_front() {
                        return Ok(Some((
                            item,
                            (client, namespace_id, filter, call, buffer, finished),
                        )));
                    }
                    if finished {
                        return Ok(None);
                    }
                    let page = client
                        .schedules()
                        .list(&namespace_id, filter.clone(), call.clone())
                        .await?;
                    if page.next_cursor.is_some() && page.next_cursor == filter.cursor {
                        return Err(protocol());
                    }
                    finished = page.next_cursor.is_none();
                    filter.cursor = page.next_cursor;
                    buffer = page.items.into();
                }
            },
        )
    }

    /// Replaces the spec, the policies and the action as a new revision; a stale
    /// `expected_revision` (at least 1) is `CONFLICT` with the reason `REVISION_CONFLICT`.
    pub async fn update(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        write: &ScheduleWrite,
        expected_revision: u64,
        call: CallOptions,
    ) -> Result<Schedule, SagatError> {
        let revision = positive_revision(expected_revision)?;
        let mut body = write.body()?;
        body["expectedRevision"] = json!(revision);
        let req = request(
            Method::PUT,
            &path(namespace_id, Some(schedule_id), &[])?,
            200,
            Vec::new(),
        )
        .json(&body)?
        .keyed(key(&call)?);
        self.client.execute(req, call).await?.parse(Schedule::parse)
    }

    pub async fn delete(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        expected_revision: u64,
        call: CallOptions,
    ) -> Result<(), SagatError> {
        let revision = positive_revision(expected_revision)?;
        let req = request(
            Method::DELETE,
            &path(namespace_id, Some(schedule_id), &[])?,
            204,
            vec![("expectedRevision".into(), revision.to_string())],
        )
        .keyed(key(&call)?);
        self.client
            .execute(req, call)
            .await
            .and_then(|reply| reply.parse(|_| Ok(())))
    }

    async fn noted(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        verb: &str,
        note: &str,
        call: CallOptions,
    ) -> Result<Schedule, SagatError> {
        let body = note_of(note)?;
        let req = request(
            Method::POST,
            &path(namespace_id, Some(schedule_id), &[verb])?,
            200,
            Vec::new(),
        )
        .json(&body)?
        .keyed(key(&call)?);
        self.client.execute(req, call).await?.parse(Schedule::parse)
    }

    /// Pauses the schedule with a non-empty note saying why.
    pub async fn pause(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        note: &str,
        call: CallOptions,
    ) -> Result<Schedule, SagatError> {
        self.noted(namespace_id, schedule_id, "pause", note, call)
            .await
    }

    pub async fn resume(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        note: &str,
        call: CallOptions,
    ) -> Result<Schedule, SagatError> {
        self.noted(namespace_id, schedule_id, "resume", note, call)
            .await
    }

    /// Acts on the schedule now, also while it is paused. The answer says what became of the action.
    /// `overlap` is for this action only.
    pub async fn trigger(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        overlap: Option<OverlapPolicy>,
        call: CallOptions,
    ) -> Result<ScheduleActionRecord, SagatError> {
        let mut body = json!({});
        if let Some(overlap) = overlap {
            body["overlap"] = json!(overlap.as_str());
        }
        let req = request(
            Method::POST,
            &path(namespace_id, Some(schedule_id), &["trigger"])?,
            202,
            Vec::new(),
        )
        .json(&body)?
        .keyed(key(&call)?);
        self.client
            .execute(req, call)
            .await?
            .parse(ScheduleActionRecord::parse)
    }

    /// Acts on every nominal time of the past range `[start, end)` that has no action yet. The
    /// instants are given as a date-time or as RFC 3339 text and are sent in UTC.
    pub async fn backfill(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        start: impl Into<Moment>,
        end: impl Into<Moment>,
        overlap: Option<OverlapPolicy>,
        call: CallOptions,
    ) -> Result<ScheduleBackfill, SagatError> {
        let mut body =
            json!({"start":start.into().utc_text("start")?,"end":end.into().utc_text("end")?});
        if let Some(overlap) = overlap {
            body["overlap"] = json!(overlap.as_str());
        }
        let req = request(
            Method::POST,
            &path(namespace_id, Some(schedule_id), &["backfills"])?,
            202,
            Vec::new(),
        )
        .json(&body)?
        .keyed(key(&call)?);
        self.client
            .execute(req, call)
            .await?
            .parse(ScheduleBackfill::parse)
    }

    pub async fn cancel_backfill(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        backfill_id: &str,
        call: CallOptions,
    ) -> Result<ScheduleBackfill, SagatError> {
        let backfill = id_segment(backfill_id, "a backfill id")?;
        let req = request(
            Method::POST,
            &path(
                namespace_id,
                Some(schedule_id),
                &["backfills", &backfill, "cancel"],
            )?,
            200,
            Vec::new(),
        )
        .keyed(key(&call)?);
        self.client
            .execute(req, call)
            .await?
            .parse(ScheduleBackfill::parse)
    }

    /// At most the latest 100 backfills of the schedule.
    pub async fn list_backfills(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        call: CallOptions,
    ) -> Result<Vec<ScheduleBackfill>, SagatError> {
        let req = request(
            Method::GET,
            &path(namespace_id, Some(schedule_id), &["backfills"])?,
            200,
            Vec::new(),
        );
        self.client.execute(req, call).await?.parse(|raw| {
            Fields::of(&raw, "a list of backfills")?
                .values("items")
                .iter()
                .map(ScheduleBackfill::read)
                .collect()
        })
    }

    /// The decisions the schedule has made, newest first.
    pub async fn list_actions(
        &self,
        namespace_id: &str,
        schedule_id: &str,
        filter: ScheduleActionFilter,
        call: CallOptions,
    ) -> Result<Page<ScheduleActionRecord>, SagatError> {
        let mut query = Vec::new();
        push_page(&mut query, filter.limit, filter.cursor.as_deref())?;
        if let Some(outcome) = &filter.outcome {
            query.push(("outcome".into(), outcome.clone()));
        }
        let req = request(
            Method::GET,
            &path(namespace_id, Some(schedule_id), &["actions"])?,
            200,
            query,
        );
        self.client
            .execute(req, call)
            .await?
            .parse(|raw| wire::page_of(raw, ScheduleActionRecord::read))
    }
}

impl Schedule {
    fn read(raw: &Value) -> Result<Self, SagatError> {
        Self::parse(raw.clone())
    }
}
