//! The answers `sagat worker dev` gives: a file that says, for each activity, what it returns or how
//! it fails, so that a workflow can be run (and its failures rehearsed) before any worker exists.
//!
//! ```yaml
//! delayMs: 0                       # how long each call takes, at most 10000
//! activities:
//!   greeting.compose:
//!     output: { greeting: Hello }  # a fixed output
//!   greeting.deliver:
//!     echo: true                   # the output is the input
//!   payments.charge:
//!     sequence:                    # one answer for each call, the last one repeats
//!       - fail: { code: PROVIDER_TIMEOUT, retryable: true }
//!       - output: { chargeId: ch-1 }
//! compensations:
//!   inventory.release:
//!     output: { released: true }   # a compensation not listed here succeeds
//! reconcilers:
//!   payments.find-charge:
//!     effect: not_applied          # applied (with an output) | not_applied | unknown
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{Value, json};

/// The longest an answer may take: a worker that sits on a task longer than its lease is a worker
/// that loses it, and a mock is not worth a heartbeat.
pub const MAX_DELAY_MS: u64 = 10_000;
/// Most activities a mock file may name (a session advertises at most 256).
pub const MAX_NAMES: usize = 200;

/// What kind of work a task is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Forward,
    Compensation,
    Reconciliation,
}

impl Kind {
    /// From the `task_kind` of a lease: empty is forward.
    #[must_use]
    pub fn of(task_kind: &str) -> Option<Self> {
        match task_kind {
            "" | "FORWARD" => Some(Self::Forward),
            "COMPENSATION" => Some(Self::Compensation),
            "RECONCILIATION" => Some(Self::Reconciliation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fail {
    pub code: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub echo: Option<bool>,
    #[serde(default)]
    pub fail: Option<Fail>,
    /// Take the task and never answer, as a worker that died would: the lease runs out.
    #[serde(default)]
    pub lose: Option<bool>,
    #[serde(default)]
    pub delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub echo: Option<bool>,
    #[serde(default)]
    pub fail: Option<Fail>,
    #[serde(default)]
    pub lose: Option<bool>,
    #[serde(default)]
    pub delay_ms: Option<u64>,
    #[serde(default)]
    pub sequence: Option<Vec<Step>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reconciler {
    pub effect: String,
    #[serde(default)]
    pub output: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Mocks {
    #[serde(default)]
    pub delay_ms: Option<u64>,
    #[serde(default)]
    pub activities: BTreeMap<String, Entry>,
    #[serde(default)]
    pub compensations: BTreeMap<String, Entry>,
    #[serde(default)]
    pub reconcilers: BTreeMap<String, Reconciler>,
    #[serde(skip)]
    calls: Mutex<HashMap<(Kind, String), usize>>,
}

/// What a task is answered with.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Output(Value),
    Echo,
    /// Never answered: the worker "died" with the task.
    Lose,
    Fail {
        code: String,
        retryable: bool,
        message: String,
    },
}

impl Mocks {
    /// Reads and checks a mock file.
    ///
    /// # Errors
    /// Every problem found, so that they are fixed together.
    pub fn parse(text: &str, yaml: bool) -> Result<Self, Vec<String>> {
        let value: Value = if yaml {
            crate::manifest::yaml_to_value(text).map_err(|e| vec![e])?
        } else {
            serde_json::from_str(text).map_err(|e| vec![e.to_string()])?
        };
        let mocks: Self = serde_json::from_value(value)
            .map_err(|e| vec![crate::manifest::quiet(&e.to_string())])?;
        let problems = mocks.problems();
        if problems.is_empty() {
            Ok(mocks)
        } else {
            Err(problems)
        }
    }

    fn problems(&self) -> Vec<String> {
        let mut found = Vec::new();
        let too_slow = |ms: Option<u64>| ms.is_some_and(|ms| ms > MAX_DELAY_MS);
        if too_slow(self.delay_ms) {
            found.push(format!("delayMs is at most {MAX_DELAY_MS}"));
        }
        if self.names().len() > MAX_NAMES {
            found.push(format!("a mock file names at most {MAX_NAMES} activities"));
        }
        for (section, entries) in [
            ("activities", &self.activities),
            ("compensations", &self.compensations),
        ] {
            for (name, entry) in entries {
                let at = format!("{section}.{name}");
                if crate::mocks::bad_name(name) {
                    found.push(format!("{at}: not an activity name"));
                }
                let direct = usize::from(entry.output.is_some())
                    + usize::from(entry.echo == Some(true))
                    + usize::from(entry.fail.is_some())
                    + usize::from(entry.lose == Some(true));
                match (&entry.sequence, direct) {
                    (Some(_), n) if n > 0 => found.push(format!(
                        "{at}: a sequence and an answer of its own, say one or the other"
                    )),
                    (Some(steps), _) if steps.is_empty() => {
                        found.push(format!("{at}: a sequence has at least one answer"));
                    }
                    (Some(steps), _) => {
                        for (i, step) in steps.iter().enumerate() {
                            let n = usize::from(step.output.is_some())
                                + usize::from(step.echo == Some(true))
                                + usize::from(step.fail.is_some())
                                + usize::from(step.lose == Some(true));
                            if n != 1 {
                                found.push(format!(
                                    "{at}.sequence[{i}]: exactly one of output, echo, fail, lose"
                                ));
                            }
                            if too_slow(step.delay_ms) {
                                found.push(format!(
                                    "{at}.sequence[{i}]: delayMs is at most {MAX_DELAY_MS}"
                                ));
                            }
                        }
                    }
                    (None, 1) => {}
                    (None, _) => found.push(format!(
                        "{at}: exactly one of output, echo, fail, lose (or a sequence)"
                    )),
                }
                if too_slow(entry.delay_ms) {
                    found.push(format!("{at}: delayMs is at most {MAX_DELAY_MS}"));
                }
            }
        }
        for (name, reconciler) in &self.reconcilers {
            let at = format!("reconcilers.{name}");
            if crate::mocks::bad_name(name) {
                found.push(format!("{at}: not an activity name"));
            }
            match reconciler.effect.as_str() {
                "applied" | "not_applied" | "unknown" => {}
                other => found.push(format!(
                    "{at}: effect is applied, not_applied or unknown, not '{}'",
                    crate::one_line(other)
                )),
            }
            if reconciler.output.is_some() && reconciler.effect != "applied" {
                found.push(format!("{at}: only an applied effect has an output"));
            }
        }
        found
    }

    /// Every activity name the file gives an answer for.
    #[must_use]
    pub fn names(&self) -> BTreeSet<String> {
        self.activities
            .keys()
            .chain(self.compensations.keys())
            .chain(self.reconcilers.keys())
            .cloned()
            .collect()
    }

    /// How long a call takes: the entry's own, or the file's.
    fn delay(&self, own: Option<u64>) -> u64 {
        own.or(self.delay_ms).unwrap_or(0).min(MAX_DELAY_MS)
    }

    /// The answer to the next call of `name` as `kind`, and how long to take over it. The calls
    /// are counted for each name: a sequence advances with every call, whichever run makes it.
    pub fn answer(&self, kind: Kind, name: &str) -> (Answer, u64) {
        let call = {
            let mut calls = self
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let slot = calls.entry((kind, name.to_owned())).or_insert(0);
            let call = *slot;
            *slot += 1;
            call
        };
        match kind {
            Kind::Forward => match self.activities.get(name) {
                Some(entry) => self.of_entry(entry, call),
                None => (
                    Answer::Fail {
                        code: "NO_MOCK".to_owned(),
                        retryable: false,
                        message: format!(
                            "no answer for the activity '{name}' in the mock file (it has: {})",
                            self.activities
                                .keys()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    },
                    0,
                ),
            },
            Kind::Compensation => match self.compensations.get(name) {
                Some(entry) => self.of_entry(entry, call),
                None => (Answer::Output(json!({})), self.delay(None)),
            },
            Kind::Reconciliation => {
                let output = match self.reconcilers.get(name) {
                    Some(r) => {
                        let mut out = json!({ "effect": r.effect });
                        if let Some(extra) = &r.output {
                            out["output"] = extra.clone();
                        }
                        out
                    }
                    None => json!({ "effect": "unknown" }),
                };
                (Answer::Output(output), self.delay(None))
            }
        }
    }

    fn of_entry(&self, entry: &Entry, call: usize) -> (Answer, u64) {
        if let Some(steps) = &entry.sequence {
            let step = &steps[call.min(steps.len() - 1)];
            return (
                step_answer(
                    step.output.as_ref(),
                    step.echo,
                    step.fail.as_ref(),
                    step.lose,
                ),
                self.delay(step.delay_ms),
            );
        }
        (
            step_answer(
                entry.output.as_ref(),
                entry.echo,
                entry.fail.as_ref(),
                entry.lose,
            ),
            self.delay(entry.delay_ms),
        )
    }
}

fn step_answer(
    output: Option<&Value>,
    echo: Option<bool>,
    fail: Option<&Fail>,
    lose: Option<bool>,
) -> Answer {
    if lose == Some(true) {
        return Answer::Lose;
    }
    if let Some(fail) = fail {
        return Answer::Fail {
            code: fail.code.clone(),
            retryable: fail.retryable,
            message: fail
                .message
                .clone()
                .unwrap_or_else(|| "an error injected by the mock file".to_owned()),
        };
    }
    if echo == Some(true) {
        return Answer::Echo;
    }
    Answer::Output(output.cloned().unwrap_or_else(|| json!({})))
}

/// Not what the platform takes as the name of an activity.
pub(crate) fn bad_name(name: &str) -> bool {
    name.is_empty()
        || name.len() > 128
        || !name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// The activities a workflow document uses: the ones it runs, the ones that undo them and the ones
/// that look up whether an effect happened.
#[must_use]
pub fn activities_of(document: &Value) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for node in document["nodes"].as_array().into_iter().flatten() {
        for key in ["activity", "compensateWith", "reconcileWith"] {
            if let Some(name) = node[key].as_str() {
                found.insert(name.to_owned());
            }
        }
    }
    found
}
