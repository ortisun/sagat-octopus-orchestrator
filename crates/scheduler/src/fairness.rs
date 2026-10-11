//! Fair selection of the work a pass takes (`FR-010-002`).
//!
//! A pass asks the store for the oldest candidates. When one tenant has a burst of them, the oldest
//! `batch` are all its own, and every other tenant waits until the burst has drained: oldest-first
//! is fair in time and unfair between tenants. This module changes *what a pass takes*, not what the
//! store keeps:
//!
//! 1. If the first page is **short** (fewer candidates than the batch), everything was taken and
//!    nobody can be hiding behind anybody: that is the common case, and it costs one query.
//! 2. Otherwise the pass **discovers** the tenants that have work. A tenant that already shows more
//!    candidates than its share of the batch is left out of the next query (the exclusion list), so
//!    that the next page shows who else has work. At most [`ROUNDS`] pages are read.
//! 3. The batch is then **shared out by weighted max-min fairness** over what each tenant showed:
//!    a tenant that has less than its share gets all it has, and what it leaves goes to the others
//!    in proportion to their weights. Inside a tenant the oldest candidates go first.
//!
//! The policy is work-conserving (a lone tenant with a burst still gets the whole batch) and
//! measurable, which is what `FR-010-002` asks for: a tenant of weight `w` that is found in a pass
//! gets at least `floor(batch * w / W)` candidates, `W` being the weight of the tenants found, or
//! all it has if that is less. It is found unless more than [`ROUNDS`] tenants that each have more
//! than their share were found before it (`scheduler_fairness_exhausted_total` counts the passes in
//! which the pages ran out). The share is a **budget per pass**, not a rate limit: the quotas of a
//! tenant (active runs, per namespace) stay where they were.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::hash::Hash;

use sagat_application::error::StoreError;
use sagat_domain::identity::TenantId;

/// The pages one selection may read (the first one included): the number of tenants it can
/// discover behind tenants that have more than their share.
pub const ROUNDS: usize = 16;
/// The weight of a tenant nobody has weighted.
pub const DEFAULT_WEIGHT: u16 = 1;

/// The scheduling weights of the tenants that have one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Weights(BTreeMap<TenantId, u16>);

impl Weights {
    #[must_use]
    pub const fn new(weights: BTreeMap<TenantId, u16>) -> Self {
        Self(weights)
    }

    /// The weight of `tenant`: 1 unless somebody gave it another.
    #[must_use]
    pub fn of(&self, tenant: TenantId) -> u16 {
        self.0
            .get(&tenant)
            .copied()
            .unwrap_or(DEFAULT_WEIGHT)
            .max(1)
    }
}

/// What a selection did, for the metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Selection {
    /// Pages read.
    pub pages: usize,
    /// Tenants that were given less than they showed.
    pub limited: usize,
    /// The pages ran out while the last one was still full: there may be tenants not found.
    pub exhausted: bool,
}

/// Shares `budget` between demands by weighted max-min fairness: a demand that is no more than its
/// share of what is left gets all of it; the rest is split by weight (the remainder, one candidate
/// each, to the heaviest first, then in the order given). Returns what each demand is given.
///
/// `demands` is `(weight, demand)`; the answer is in the same order, and never more than the budget.
#[must_use]
pub fn share(budget: u32, demands: &[(u16, u32)]) -> Vec<u32> {
    let mut given = vec![0_u32; demands.len()];
    let mut left = u64::from(budget);
    let mut active: Vec<usize> = (0..demands.len()).filter(|&i| demands[i].1 > 0).collect();
    while !active.is_empty() && left > 0 {
        let total: u64 = active.iter().map(|&i| u64::from(demands[i].0)).sum();
        // Those that want no more than their share are satisfied, and leave the rest to the others.
        let (small, big): (Vec<usize>, Vec<usize>) = active
            .iter()
            .partition(|&&i| u64::from(demands[i].1) * total <= left * u64::from(demands[i].0));
        if small.is_empty() {
            let mut remainder = left;
            for &i in &active {
                let part = left * u64::from(demands[i].0) / total;
                given[i] += u32::try_from(part).unwrap_or(u32::MAX);
                remainder -= part;
            }
            let mut by_weight = active.clone();
            by_weight.sort_by_key(|&i| std::cmp::Reverse(demands[i].0));
            for i in by_weight {
                if remainder == 0 {
                    break;
                }
                // Each wants more than its share, so one more is never more than it wants.
                given[i] += 1;
                remainder -= 1;
            }
            break;
        }
        for i in small {
            given[i] += demands[i].1;
            left -= u64::from(demands[i].1);
        }
        active = big;
    }
    given
}

/// Picks up to `batch` candidates fairly. `fetch(excluded)` reads a page of the oldest candidates
/// of tenants that are not in `excluded`; it must return at most `batch` of them.
///
/// # Errors
/// Whatever `fetch` returns.
pub async fn select<T, K, F, Fut>(
    batch: u32,
    weights: &Weights,
    tenant: impl Fn(&T) -> TenantId,
    key: impl Fn(&T) -> K,
    mut fetch: F,
) -> Result<(Vec<T>, Selection), StoreError>
where
    K: Eq + Hash,
    F: FnMut(Vec<TenantId>) -> Fut,
    Fut: Future<Output = Result<Vec<T>, StoreError>>,
{
    let wanted = usize::try_from(batch).unwrap_or(usize::MAX);
    let first = fetch(Vec::new()).await?;
    let mut selection = Selection {
        pages: 1,
        ..Selection::default()
    };
    if first.len() < wanted {
        // Short page: all the candidates there are, so nobody is hiding behind anybody.
        return Ok((first, selection));
    }

    // What each tenant showed, oldest first, and the order the candidates were met in.
    let mut seen: HashSet<K> = HashSet::new();
    let mut shown: BTreeMap<TenantId, Vec<(usize, T)>> = BTreeMap::new();
    let mut order: Vec<TenantId> = Vec::new();
    let mut met = 0_usize;
    let mut excluded: Vec<TenantId> = Vec::new();
    let mut page = first;
    loop {
        let full = page.len() >= wanted;
        let mut in_page: Vec<TenantId> = Vec::new();
        for item in page {
            if !seen.insert(key(&item)) {
                continue;
            }
            let owner = tenant(&item);
            if !shown.contains_key(&owner) {
                order.push(owner);
            }
            if !in_page.contains(&owner) {
                in_page.push(owner);
            }
            shown.entry(owner).or_default().push((met, item));
            met += 1;
        }
        if !full {
            break;
        }
        // A tenant that shows at least its share of the batch cannot be given more than that
        // unless the others leave some: it is left out, to see who else there is.
        let total: u64 = order.iter().map(|t| u64::from(weights.of(*t))).sum();
        let mut progressed = false;
        for owner in &order {
            if excluded.contains(owner) {
                continue;
            }
            let rows = u64::try_from(shown.get(owner).map_or(0, Vec::len)).unwrap_or(u64::MAX);
            if rows.saturating_mul(total) >= u64::from(batch) * u64::from(weights.of(*owner)) {
                excluded.push(*owner);
                progressed = true;
            }
        }
        if !progressed {
            // Nobody on this page has its share yet, and asking again would show the same page:
            // leave out everybody on it. (This is a page of many tenants with little each.)
            for owner in in_page {
                if !excluded.contains(&owner) {
                    excluded.push(owner);
                    progressed = true;
                }
            }
        }
        if !progressed {
            break;
        }
        if selection.pages >= ROUNDS {
            selection.exhausted = true;
            break;
        }
        page = fetch(excluded.clone()).await?;
        selection.pages += 1;
    }

    let demands: Vec<(u16, u32)> = order
        .iter()
        .map(|t| {
            (
                weights.of(*t),
                u32::try_from(shown.get(t).map_or(0, Vec::len)).unwrap_or(u32::MAX),
            )
        })
        .collect();
    let given = share(batch, &demands);
    selection.limited = given
        .iter()
        .zip(&demands)
        .filter(|(given, (_, demand))| given < &demand)
        .count();
    let mut chosen: Vec<(usize, T)> = Vec::with_capacity(wanted);
    for (owner, count) in order.iter().zip(given) {
        if let Some(items) = shown.remove(owner) {
            chosen.extend(
                items
                    .into_iter()
                    .take(usize::try_from(count).unwrap_or(usize::MAX)),
            );
        }
    }
    // Back in the order they were met: the oldest of the first page first.
    chosen.sort_by_key(|(met, _)| *met);
    Ok((
        chosen.into_iter().map(|(_, item)| item).collect(),
        selection,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    type Item = (TenantId, u32);
    type Page = std::future::Ready<Result<Vec<Item>, StoreError>>;

    fn tenant(n: u128) -> TenantId {
        TenantId::new(Uuid::from_u128(n))
    }

    /// A store of due items in age order, minus the excluded tenants.
    fn fetcher(items: Vec<Item>, batch: u32) -> impl FnMut(Vec<TenantId>) -> Page {
        move |excluded| {
            std::future::ready(Ok(items
                .iter()
                .filter(|(t, _)| !excluded.contains(t))
                .take(usize::try_from(batch).unwrap())
                .copied()
                .collect()))
        }
    }

    async fn pick(items: Vec<Item>, batch: u32, weights: &Weights) -> (Vec<Item>, Selection) {
        select(
            batch,
            weights,
            |i: &Item| i.0,
            |i| i.1,
            fetcher(items, batch),
        )
        .await
        .unwrap()
    }

    fn count(picked: &[Item], of: TenantId) -> usize {
        picked.iter().filter(|(t, _)| *t == of).count()
    }

    #[tokio::test]
    async fn a_short_first_page_is_taken_whole_in_one_query() {
        let items = (0..10).map(|n| (tenant(1), n)).collect();
        let (picked, selection) = pick(items, 200, &Weights::default()).await;
        assert_eq!(picked.len(), 10);
        assert_eq!(selection.pages, 1);
        assert_eq!(selection.limited, 0);
    }

    #[tokio::test]
    async fn a_burst_does_not_hide_a_tenant_with_a_little_work() {
        // A has 5 000 candidates older than B's five.
        let mut items: Vec<_> = (0..5_000).map(|n| (tenant(1), n)).collect();
        items.extend((5_000..5_005).map(|n| (tenant(2), n)));
        let (picked, selection) = pick(items, 200, &Weights::default()).await;
        assert_eq!(picked.len(), 200, "the batch is used in full");
        assert_eq!(count(&picked, tenant(2)), 5, "B is served in the same pass");
        assert_eq!(count(&picked, tenant(1)), 195, "A gets the rest");
        assert_eq!(selection.limited, 1, "only A was held back");
    }

    #[tokio::test]
    async fn a_lone_tenant_still_gets_the_whole_batch_oldest_first() {
        let items = (0..1_000).map(|n| (tenant(1), n)).collect();
        let (picked, _) = pick(items, 200, &Weights::default()).await;
        let serials: Vec<u32> = picked.iter().map(|(_, s)| *s).collect();
        assert_eq!(serials, (0..200).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn two_tenants_with_a_lot_each_share_the_batch_by_their_weights() {
        // A's candidates are all older than B's; B has weight 3.
        let weights = Weights::new(BTreeMap::from([(tenant(2), 3)]));
        let mut items: Vec<_> = (0..1_000).map(|n| (tenant(1), n)).collect();
        items.extend((1_000..2_000).map(|n| (tenant(2), n)));
        let (picked, selection) = pick(items, 200, &weights).await;
        assert_eq!(picked.len(), 200);
        assert_eq!(count(&picked, tenant(1)), 50);
        assert_eq!(count(&picked, tenant(2)), 150);
        assert_eq!(selection.limited, 2);
    }

    #[tokio::test]
    async fn with_equal_weights_each_gets_the_same() {
        let mut items: Vec<_> = (0..1_000).map(|n| (tenant(1), n)).collect();
        items.extend((1_000..2_000).map(|n| (tenant(2), n)));
        items.extend((2_000..3_000).map(|n| (tenant(3), n)));
        let (picked, _) = pick(items, 30, &Weights::default()).await;
        assert_eq!(count(&picked, tenant(1)), 10);
        assert_eq!(count(&picked, tenant(2)), 10);
        assert_eq!(count(&picked, tenant(3)), 10);
    }

    #[tokio::test]
    async fn every_tenant_found_is_served_when_there_are_many() {
        let items: Vec<_> = (0..2_000)
            .map(|n| (tenant(1 + u128::from(n % 20)), n))
            .collect();
        let (picked, _) = pick(items, 40, &Weights::default()).await;
        assert_eq!(picked.len(), 40);
        for t in 1..=20 {
            assert_eq!(count(&picked, tenant(t)), 2);
        }
    }

    #[tokio::test]
    async fn the_pages_are_bounded_and_the_pass_says_when_they_ran_out() {
        // Fifty tenants whose work comes in blocks of a hundred, oldest block first: each page
        // shows one tenant only, so finding the next one costs a page.
        let items: Vec<_> = (0..5_000)
            .map(|n| (tenant(1 + u128::from(n / 100)), n))
            .collect();
        let (picked, selection) = pick(items, 40, &Weights::default()).await;
        assert_eq!(picked.len(), 40, "the batch is used in full");
        assert_eq!(selection.pages, ROUNDS);
        assert!(selection.exhausted);
        let served = (1..=50).filter(|t| count(&picked, tenant(*t)) > 0).count();
        assert_eq!(served, ROUNDS, "one tenant per page found, each served");
    }

    #[tokio::test]
    async fn a_page_of_many_tenants_with_little_each_still_makes_progress() {
        // 300 tenants, one candidate each, batch 100: the pass must end and use the batch.
        let items: Vec<_> = (0..300).map(|n| (tenant(1 + u128::from(n)), n)).collect();
        let (picked, selection) = pick(items, 100, &Weights::default()).await;
        assert_eq!(picked.len(), 100);
        assert!(selection.pages <= ROUNDS);
    }

    #[test]
    fn sharing_is_weighted_max_min() {
        // Nobody wants less than its share: split by weight, the remainder to the heaviest.
        assert_eq!(share(32, &[(1, 99), (1, 99), (4, 99)]), vec![5, 5, 22]);
        // One wants little: the others take what it leaves.
        assert_eq!(share(200, &[(1, 1_000), (1, 5)]), vec![195, 5]);
        // Chains: after the small one leaves, the next is small against the new share.
        assert_eq!(
            share(100, &[(1, 10), (1, 40), (1, 1_000)]),
            vec![10, 40, 50]
        );
        // Less demand than budget: everybody gets what it asked for and no more.
        assert_eq!(share(100, &[(2, 10), (1, 20)]), vec![10, 20]);
        // Nothing to share, nothing asked.
        assert_eq!(share(0, &[(1, 5)]), vec![0]);
        assert_eq!(share(10, &[]), Vec::<u32>::new());
        assert_eq!(share(10, &[(1, 0), (1, 3)]), vec![0, 3]);
    }

    #[test]
    fn sharing_never_gives_more_than_the_budget_or_the_demand() {
        for budget in [1_u32, 7, 32, 200] {
            let demands = [(1_u16, 3_u32), (2, 50), (16, 1_000), (1, 0), (3, 9)];
            let given = share(budget, &demands);
            assert!(given.iter().sum::<u32>() <= budget);
            for (given, (_, demand)) in given.iter().zip(&demands) {
                assert!(given <= demand);
            }
        }
    }
}
