//! The operator chain of a `forEach` applied at scope entry (design 042, section 3; ADR-0087).
//!
//! A chain is applied once, by one call of [`crate::execution::ExpressionEvaluator::collection_chain`], over the list
//! `items` yields or the elements a blob source holds. The call builds the run's environment once and evaluates stage by
//! stage, every element of a stage's input before the next stage starts, under the running ceilings of
//! [`ChainCeilings`]. Its answer is either the chain's output or a [`ChainFailure`] that names the stage, the operator
//! and the position, never a value. The scheduler records the output with the entry transaction; nothing evaluates the
//! chain again once that commits (FR-042-009).

use std::fmt;
use std::time::Duration;

use sagat_domain::blobs::BlobDescriptor;
use sagat_domain::execution::FailureCode;
use sagat_domain::operators::OperatorName;
use sagat_domain::workflow::Digest;

use crate::execution::{MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES};

/// The most elements a chain's input may have: the ceiling published for collections in 0.3.0 (OD-2026-11-13-1,
/// FR-042-007 (a)). A blob source is materialised with this bound instead of the node's `maxItems` when it has a chain.
pub const MAX_CHAIN_INPUT: usize = 1_000;

/// The most elements one stage may produce (FR-042-007 (b)).
pub const MAX_STAGE_ELEMENTS: usize = 1_000;

/// The cooperative wall-time budget of one chain, checked between elements (FR-042-006).
pub const CHAIN_WALL_TIME: Duration = Duration::from_secs(1);

/// The most lists and maps an element may nest. The JSON parser of every reader of the stored items refuses 128 levels,
/// and the snapshot array, a task's input and a collection's result wrap an item in a few more, so an element is held
/// to 120 and refused when produced rather than stored and then refused by every read (design 042 review).
pub const MAX_ELEMENT_DEPTH: usize = 120;

/// The running ceilings of a chain. Each one maps to exactly one code (design 042, section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainCeilings {
    /// Elements of the chain's input.
    pub max_input: usize,
    /// Elements of one stage's output.
    pub max_stage_elements: usize,
    /// Serialised bytes of one element, as compact JSON.
    pub max_element_bytes: usize,
    /// Serialised bytes of one stage's output in all: the sum of its elements' bytes, as the items of a blob source are
    /// counted (`canonical_items`).
    pub max_stage_bytes: usize,
    /// Wall time of the whole chain, checked between elements.
    pub wall_time: Duration,
}

impl ChainCeilings {
    /// The ceilings of a collection chain in release 0.3.0 (FR-042-007).
    pub const COLLECTION: Self = Self {
        max_input: MAX_CHAIN_INPUT,
        max_stage_elements: MAX_STAGE_ELEMENTS,
        max_element_bytes: MAX_PAYLOAD_BYTES,
        max_stage_bytes: MAX_RUN_OUTPUT_BYTES,
        wall_time: CHAIN_WALL_TIME,
    };
}

impl Default for ChainCeilings {
    fn default() -> Self {
        Self::COLLECTION
    }
}

/// What the chain runs over.
#[derive(Debug, Clone, Copy)]
pub enum ChainInput<'a> {
    /// The node's `items` expression, evaluated in the chain's own environment. It may yield a blob reference instead
    /// of a list, and then the answer is [`ChainAnswer::Blob`].
    Items(&'a sagat_domain::workflow::Expression),
    /// The elements a blob source holds, as canonical compact JSON, already materialised.
    Elements(&'a [Vec<u8>]),
}

/// How many elements a stage took and gave, for `operator_elements_total` (FR-042-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageTally {
    pub operator: OperatorName,
    pub input: u32,
    pub output: u32,
}

impl StageTally {
    /// The `(outcome, count)` pairs this stage counts: `kept` and `dropped` for the operators that select, `produced`
    /// for those that build new elements.
    #[must_use]
    pub fn outcomes(&self) -> Vec<(&'static str, u64)> {
        match self.operator {
            OperatorName::Map
            | OperatorName::FlatMap
            | OperatorName::Window
            | OperatorName::Debounce
            | OperatorName::IgnoreElements => vec![("produced", u64::from(self.output))],
            OperatorName::Filter
            | OperatorName::Skip
            | OperatorName::Take
            | OperatorName::Distinct => vec![
                ("kept", u64::from(self.output)),
                ("dropped", u64::from(self.input.saturating_sub(self.output))),
            ],
        }
    }
}

/// The output of a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainOutput {
    /// Elements of the chain's input.
    pub source_count: u32,
    /// The output, each element as canonical compact JSON, in order: the collection's membership.
    pub items: Vec<Vec<u8>>,
    /// SHA-256 of the chain's canonical encoding (D042-100), what a chained manifest records as `chainSha256`.
    pub chain_digest: Digest,
    /// One tally per stage, in stage order.
    pub stages: Vec<StageTally>,
}

/// What a chain call answers when it did not fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainAnswer {
    Output(ChainOutput),
    /// `items` yielded a blob reference: the scheduler materialises it, bounded by [`ChainCeilings::max_input`], and
    /// calls the chain again with [`ChainInput::Elements`].
    Blob(BlobDescriptor),
}

/// Which running ceiling was crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainCeiling {
    InputElements,
    StageElements,
    ElementBytes,
    StageBytes,
    WallTime,
    /// An element nested deeper than [`MAX_ELEMENT_DEPTH`] levels.
    ElementDepth,
}

impl ChainCeiling {
    /// The `reason` label of `scheduler_collection_source_failures_total`.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::InputElements => "operator_input",
            Self::StageElements => "operator_stage_elements",
            Self::ElementBytes => "operator_element_bytes",
            Self::StageBytes => "operator_stage_bytes",
            Self::WallTime => "wall_time",
            Self::ElementDepth => "operator_element_depth",
        }
    }
}

/// Why a stage's expression could not be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageFault {
    /// The expression failed to evaluate (a missing field, a type error, a budget of the language).
    Expression,
    /// A `filter`'s `where` gave something other than a boolean.
    NotBoolean,
    /// A `flatMap`'s `select` gave something other than a list.
    NotAList,
}

impl StageFault {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Expression => "expression",
            Self::NotBoolean => "not_boolean",
            Self::NotAList => "not_a_list",
        }
    }
}

/// Why a chain failed at entry. A closed set the scheduler maps to one code each; it names a stage, an operator and a
/// position, never a value (FR-042-006, D042-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainFailure {
    /// The `items` expression failed, or yielded neither a list nor a blob reference.
    Items,
    /// The evaluator does not apply chains (an adapter that predates them): failed closed.
    Unsupported,
    /// A running ceiling was crossed. `stage` is `None` for the chain's input; `position` is the position, in the
    /// stage's input, of the element whose output crossed it (or that was next when the time ran out).
    Budget {
        ceiling: ChainCeiling,
        stage: Option<usize>,
        operator: Option<OperatorName>,
        position: Option<usize>,
    },
    /// The first failing stage, then the first failing position in it.
    Stage {
        stage: usize,
        operator: OperatorName,
        position: usize,
        fault: StageFault,
    },
}

impl ChainFailure {
    /// The run's failure code (design 042, section 3).
    #[must_use]
    pub fn code(&self) -> FailureCode {
        match self {
            Self::Items | Self::Unsupported | Self::Stage { .. } => FailureCode::expression_error(),
            Self::Budget { .. } => FailureCode::collection_operator_budget(),
        }
    }

    /// The stage, the operator and the position the failure names, for structured log fields.
    #[must_use]
    pub const fn location(&self) -> (Option<usize>, Option<OperatorName>, Option<usize>) {
        match *self {
            Self::Items | Self::Unsupported => (None, None, None),
            Self::Budget {
                stage,
                operator,
                position,
                ..
            } => (stage, operator, position),
            Self::Stage {
                stage,
                operator,
                position,
                ..
            } => (Some(stage), Some(operator), Some(position)),
        }
    }

    /// The `reason` label of `scheduler_collection_source_failures_total`: a closed set.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Items => "expression",
            Self::Unsupported => "operator_unsupported",
            Self::Budget { ceiling, .. } => ceiling.reason(),
            Self::Stage { fault, .. } => match fault {
                StageFault::Expression => "operator_expression",
                StageFault::NotBoolean => "operator_not_boolean",
                StageFault::NotAList => "operator_not_a_list",
            },
        }
    }
}

impl fmt::Display for ChainFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Items => f.write_str("the collection items expression failed"),
            Self::Unsupported => f.write_str("this evaluator does not apply operator chains"),
            Self::Budget {
                ceiling,
                stage: None,
                ..
            } => write!(
                f,
                "the chain's input crossed its ceiling ({})",
                ceiling.reason()
            ),
            Self::Budget {
                ceiling,
                stage: Some(stage),
                operator,
                position,
            } => {
                write!(f, "operators[{stage}]")?;
                if let Some(operator) = operator {
                    write!(f, " ({operator})")?;
                }
                if let Some(position) = position {
                    write!(f, " at position {position}")?;
                }
                write!(f, " crossed a ceiling ({})", ceiling.reason())
            }
            Self::Stage {
                stage,
                operator,
                position,
                fault,
            } => write!(
                f,
                "operators[{stage}] ({operator}) failed at position {position} ({})",
                fault.as_str()
            ),
        }
    }
}

impl std::error::Error for ChainFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ceiling_is_one_budget_code_and_every_stage_fault_an_expression_error() {
        for ceiling in [
            ChainCeiling::InputElements,
            ChainCeiling::StageElements,
            ChainCeiling::ElementBytes,
            ChainCeiling::StageBytes,
            ChainCeiling::WallTime,
            ChainCeiling::ElementDepth,
        ] {
            let failure = ChainFailure::Budget {
                ceiling,
                stage: Some(2),
                operator: Some(OperatorName::Map),
                position: Some(7),
            };
            assert_eq!(failure.code().as_str(), "COLLECTION_OPERATOR_BUDGET");
            assert_eq!(failure.reason(), ceiling.reason());
        }
        for fault in [
            StageFault::Expression,
            StageFault::NotBoolean,
            StageFault::NotAList,
        ] {
            let failure = ChainFailure::Stage {
                stage: 0,
                operator: OperatorName::Filter,
                position: 3,
                fault,
            };
            assert_eq!(failure.code().as_str(), "EXPRESSION_ERROR");
        }
        assert_eq!(ChainFailure::Items.code().as_str(), "EXPRESSION_ERROR");
    }

    #[test]
    fn a_failure_names_the_stage_the_operator_and_the_position() {
        let failure = ChainFailure::Stage {
            stage: 1,
            operator: OperatorName::FlatMap,
            position: 4,
            fault: StageFault::NotAList,
        };
        assert_eq!(
            failure.to_string(),
            "operators[1] (flatMap) failed at position 4 (not_a_list)"
        );
    }

    #[test]
    fn the_collection_ceilings_are_the_published_numbers() {
        let c = ChainCeilings::COLLECTION;
        assert_eq!(
            (
                c.max_input,
                c.max_stage_elements,
                c.max_element_bytes,
                c.max_stage_bytes,
                c.wall_time
            ),
            (
                1_000,
                1_000,
                256 * 1024,
                4 * 1024 * 1024,
                Duration::from_secs(1)
            )
        );
    }

    #[test]
    fn tallies_count_what_each_operator_says() {
        let filter = StageTally {
            operator: OperatorName::Filter,
            input: 10,
            output: 4,
        };
        assert_eq!(filter.outcomes(), vec![("kept", 4), ("dropped", 6)]);
        let flat = StageTally {
            operator: OperatorName::FlatMap,
            input: 2,
            output: 9,
        };
        assert_eq!(flat.outcomes(), vec![("produced", 9)]);
    }
}
