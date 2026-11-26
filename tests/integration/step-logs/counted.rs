//! What the step-log passes count, read from a recorder of the test's own on a runtime of one thread, so that what
//! other tests count in this process does not reach the numbers read here (as `schedules-bulk-operations/counted.rs`).

use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use sagat_telemetry::Metrics;

/// What `body` counts, as the Prometheus page at its end; `body` is given the recorder to read a page midway.
pub(crate) fn recorded<F: Future<Output = ()>>(body: impl FnOnce(Rc<Metrics>) -> F) -> String {
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(60));
    let metrics = Rc::new(metrics);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    metrics::with_local_recorder(&recorder, || runtime.block_on(body(Rc::clone(&metrics))));
    metrics.render()
}

/// The value of the series of `name` whose labels are exactly `labels`, on the rendered page.
pub(crate) fn series(page: &str, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
    page.lines()
        .filter(|line| !line.starts_with('#'))
        .find(|line| {
            let Some(rest) = line.strip_prefix(name) else {
                return false;
            };
            if labels.is_empty() {
                return rest.starts_with(' ');
            }
            let Some(inside) = rest
                .strip_prefix('{')
                .and_then(|r| r.split_once('}'))
                .map(|(i, _)| i)
            else {
                return false;
            };
            let found: Vec<&str> = inside.split(',').collect();
            found.len() == labels.len()
                && labels
                    .iter()
                    .all(|(key, value)| found.contains(&format!("{key}=\"{value}\"").as_str()))
        })
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
}
