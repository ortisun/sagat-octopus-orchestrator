//! What starts a workflow other than somebody asking (`FR-004-001`).
//!
//! A trigger is part of the definition, so it is pinned to a version and the active version's
//! triggers are the ones in force. All the kinds that start a run because something *arrived* — an
//! event, a webhook, a message — reduce to the same question about the event: does its source and
//! its type match? They differ in how the author names the source: an event names no source (any
//! source can publish it), a webhook names the endpoint that received it and a message names the
//! connector that consumed it. A cron trigger starts a run because a clock says so.

use std::fmt;

use crate::identity::Slug;
use crate::workflow::{EventType, Expression, WorkflowError};

/// At most this many triggers in one definition.
pub const MAX_TRIGGERS: usize = 16;

/// Which kind of arrival (or clock) starts the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TriggerKind {
    /// Somebody starts it (the API). Recorded for completeness; nothing is registered for it.
    Manual,
    /// An event of a type, from any source.
    Event,
    /// An event received by a webhook endpoint of the namespace.
    Webhook,
    /// An event consumed from a message topic or queue by a connector of the namespace.
    Message,
    /// A schedule.
    Cron,
}

impl TriggerKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Event => "event",
            Self::Webhook => "webhook",
            Self::Message => "message",
            Self::Cron => "cron",
        }
    }

    /// Whether a run is started because an event arrived.
    #[must_use]
    pub const fn is_event(self) -> bool {
        matches!(self, Self::Event | Self::Webhook | Self::Message)
    }
}

impl fmt::Display for TriggerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TriggerKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "manual" => Self::Manual,
            "event" => Self::Event,
            "webhook" => Self::Webhook,
            "message" => Self::Message,
            "cron" => Self::Cron,
            _ => return Err(()),
        })
    }
}

/// A schedule: five fields (minute, hour, day of month, month, day of week) in a time zone.
///
/// The domain keeps the text and checks its shape; the adapter that computes the next occurrence
/// (it needs a calendar and a time-zone database) checks that it means something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    expression: String,
    timezone: String,
}

impl CronSpec {
    /// # Errors
    /// [`WorkflowError::InvalidTrigger`] when the expression does not have five fields of plain
    /// cron characters, or the time zone is not a name.
    pub fn parse(expression: &str, timezone: &str) -> Result<Self, WorkflowError> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '*' | '/' | ',' | '-' | '?');
        if fields.len() != 5
            || expression.len() > 120
            || !fields.iter().all(|f| f.chars().all(plain))
        {
            return Err(WorkflowError::InvalidTrigger(
                "a cron expression has five fields: minute hour day-of-month month day-of-week",
            ));
        }
        if timezone.is_empty()
            || timezone.len() > 64
            || !timezone
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-'))
        {
            return Err(WorkflowError::InvalidTrigger(
                "a time zone is an IANA name such as UTC or America/Sao_Paulo",
            ));
        }
        Ok(Self {
            expression: fields.join(" "),
            timezone: timezone.to_owned(),
        })
    }

    /// The expression, with its fields separated by single spaces.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    #[must_use]
    pub fn timezone(&self) -> &str {
        &self.timezone
    }
}

/// One trigger of a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    kind: TriggerKind,
    /// The endpoint (webhook) or connector (message) the event must come from; an event trigger
    /// may name a source too.
    source: Option<Slug>,
    event_type: Option<EventType>,
    /// A side-effect-free condition over the event (`event.type`, `event.source`,
    /// `event.subject`, `data.*`); the event starts a run only when it holds.
    filter: Option<Expression>,
    schedule: Option<CronSpec>,
}

impl Trigger {
    /// Somebody starts the workflow.
    #[must_use]
    pub const fn manual() -> Self {
        Self {
            kind: TriggerKind::Manual,
            source: None,
            event_type: None,
            filter: None,
            schedule: None,
        }
    }

    /// An event of `event_type`, from `source` when given.
    #[must_use]
    pub const fn event(event_type: EventType, source: Option<Slug>) -> Self {
        Self {
            kind: TriggerKind::Event,
            source,
            event_type: Some(event_type),
            filter: None,
            schedule: None,
        }
    }

    /// Any event (of `event_type` when given) received by the webhook endpoint `endpoint`.
    #[must_use]
    pub const fn webhook(endpoint: Slug, event_type: Option<EventType>) -> Self {
        Self {
            kind: TriggerKind::Webhook,
            source: Some(endpoint),
            event_type,
            filter: None,
            schedule: None,
        }
    }

    /// Any message (of `event_type` when given) consumed by the connector `connector`.
    #[must_use]
    pub const fn message(connector: Slug, event_type: Option<EventType>) -> Self {
        Self {
            kind: TriggerKind::Message,
            source: Some(connector),
            event_type,
            filter: None,
            schedule: None,
        }
    }

    /// A schedule.
    #[must_use]
    pub const fn cron(schedule: CronSpec) -> Self {
        Self {
            kind: TriggerKind::Cron,
            source: None,
            event_type: None,
            filter: None,
            schedule: Some(schedule),
        }
    }

    /// Adds a condition over the event.
    #[must_use]
    pub fn with_filter(mut self, filter: Expression) -> Self {
        self.filter = Some(filter);
        self
    }

    #[must_use]
    pub const fn kind(&self) -> TriggerKind {
        self.kind
    }

    #[must_use]
    pub const fn source(&self) -> Option<&Slug> {
        self.source.as_ref()
    }

    #[must_use]
    pub const fn event_type(&self) -> Option<&EventType> {
        self.event_type.as_ref()
    }

    #[must_use]
    pub const fn filter(&self) -> Option<&Expression> {
        self.filter.as_ref()
    }

    #[must_use]
    pub const fn schedule(&self) -> Option<&CronSpec> {
        self.schedule.as_ref()
    }

    /// Whether two triggers are about the same thing: the same kind, source, type and schedule.
    /// The filter is not part of it, so editing a filter keeps the trigger's identity (and with it
    /// the memory of which events already started a run).
    #[must_use]
    pub fn is_same_trigger(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.source == other.source
            && self.event_type == other.event_type
            && self.schedule == other.schedule
    }

    /// Whether an event with this `source` and `event_type` is one this trigger is about (before the
    /// filter, which needs the data).
    #[must_use]
    pub fn matches(&self, source: &Slug, event_type: &EventType) -> bool {
        self.kind.is_event()
            && self.source.as_ref().is_none_or(|wanted| wanted == source)
            && self
                .event_type
                .as_ref()
                .is_none_or(|wanted| wanted == event_type)
    }

    /// The shape rules of each kind.
    ///
    /// # Errors
    /// [`WorkflowError::InvalidTrigger`] naming the rule.
    pub fn validate(&self) -> Result<(), WorkflowError> {
        let bad = |why: &'static str| Err(WorkflowError::InvalidTrigger(why));
        match self.kind {
            TriggerKind::Manual
                if self.source.is_some() || self.event_type.is_some() || self.filter.is_some() =>
            {
                bad("a manual trigger has no source, type or filter")
            }
            TriggerKind::Event if self.event_type.is_none() => {
                bad("an event trigger needs an eventType")
            }
            TriggerKind::Webhook if self.source.is_none() => {
                bad("a webhook trigger needs the endpoint that receives it")
            }
            TriggerKind::Message if self.source.is_none() => {
                bad("a message trigger needs the connector that consumes it")
            }
            TriggerKind::Cron
                if self.schedule.is_none()
                    || self.source.is_some()
                    || self.event_type.is_some()
                    || self.filter.is_some() =>
            {
                bad("a cron trigger has a schedule and nothing else")
            }
            _ => Ok(()),
        }
    }
}
