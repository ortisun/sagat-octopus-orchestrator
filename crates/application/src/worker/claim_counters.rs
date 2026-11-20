//! What a poll did with its candidates, as counters (design 038, FR-038-007).
//!
//! The ratio between them is the cost of a lease: how many candidates were read for it, how many
//! were tried, and how many runs were opened and loaded. The only label is the stage, a closed set:
//! never a tenant, a namespace, an activity, a session, a worker, a pool or a run.

/// What one claim found out for its poll, beside the tasks it leased.
#[derive(Debug, Default)]
pub(super) struct ClaimProgress {
    /// The session can take nothing more, whatever the next candidate is: the poll ends.
    pub(super) session_full: bool,
    /// Runs opened under their row lock, each of which was loaded.
    pub(super) opened: u64,
}

/// `worker_claim_candidates_total{stage}`, once per poll: `fetched` by the scan, `probed` (a claim
/// transaction was begun for it), `opened` (its run was locked and loaded) and `leased` (a task was
/// returned). Stages with nothing to count are not emitted.
pub(super) fn count_claim(fetched: u64, probed: u64, opened: u64, leased: u64) {
    for (stage, amount) in [
        ("fetched", fetched),
        ("probed", probed),
        ("opened", opened),
        ("leased", leased),
    ] {
        if amount > 0 {
            metrics::counter!("worker_claim_candidates_total", "stage" => stage).increment(amount);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use metrics::{
        Counter, CounterFn, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit,
    };

    use super::count_claim;

    /// One increment of a counter: its name, its labels and the amount.
    type Seen = (String, Vec<(String, String)>, u64);

    struct Sink(Arc<Mutex<Vec<Seen>>>);
    struct Cell(Key, Arc<Mutex<Vec<Seen>>>);
    impl CounterFn for Cell {
        fn increment(&self, value: u64) {
            let labels = self
                .0
                .labels()
                .map(|label| (label.key().to_owned(), label.value().to_owned()))
                .collect();
            self.1
                .lock()
                .unwrap()
                .push((self.0.name().to_owned(), labels, value));
        }
        fn absolute(&self, _: u64) {}
    }
    impl Recorder for Sink {
        fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
            Counter::from_arc(Arc::new(Cell(key.clone(), Arc::clone(&self.0))))
        }
        fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
            Gauge::noop()
        }
        fn register_histogram(&self, _: &Key, _: &Metadata<'_>) -> Histogram {
            Histogram::noop()
        }
    }

    fn counted(fetched: u64, probed: u64, opened: u64, leased: u64) -> Vec<(String, u64)> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        metrics::with_local_recorder(&Sink(Arc::clone(&seen)), || {
            count_claim(fetched, probed, opened, leased);
        });
        let seen = seen.lock().unwrap();
        seen.iter()
            .map(|(name, labels, amount)| {
                assert_eq!(name, "worker_claim_candidates_total");
                assert_eq!(labels.len(), 1, "the stage is the only label: {labels:?}");
                assert_eq!(labels[0].0, "stage");
                (labels[0].1.clone(), *amount)
            })
            .collect()
    }

    #[test]
    fn a_poll_counts_each_stage_once_with_the_stage_as_its_only_label() {
        assert_eq!(
            counted(64, 60, 3, 2),
            [
                ("fetched".to_owned(), 64),
                ("probed".to_owned(), 60),
                ("opened".to_owned(), 3),
                ("leased".to_owned(), 2)
            ]
        );
    }

    #[test]
    fn a_stage_with_nothing_to_count_is_not_emitted() {
        assert_eq!(
            counted(4, 4, 0, 0),
            [("fetched".to_owned(), 4), ("probed".to_owned(), 4)]
        );
        assert!(counted(0, 0, 0, 0).is_empty());
    }
}
