use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

use crate::plugin::SourceId;

/// What one source's ticks have cost so far: how many ran, their time together, and the
/// longest. Wall time around the source's whole group, host calls included, which is what a
/// frame waits on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickCost {
    pub ticks: u64,
    pub total: Duration,
    pub longest: Duration,
}

/// Every source's [`TickCost`], kept for the life of the VM, so that what a plugin spends on
/// drawing every frame can be read rather than guessed at.
#[derive(Debug, Default)]
pub struct TickCosts {
    by_source: RefCell<BTreeMap<SourceId, TickCost>>,
}

impl TickCosts {
    pub fn record(&self, source: SourceId, elapsed: Duration) {
        let mut by_source = self.by_source.borrow_mut();
        let cost = by_source.entry(source).or_default();
        cost.ticks += 1;
        cost.total += elapsed;
        cost.longest = cost.longest.max(elapsed);
    }

    pub fn snapshot(&self) -> Vec<(SourceId, TickCost)> {
        self.by_source
            .borrow()
            .iter()
            .map(|(source, cost)| (*source, *cost))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tick_adds_to_its_own_source_and_the_longest_is_kept() {
        let costs = TickCosts::default();
        let a = SourceId(1);
        let b = SourceId(2);
        costs.record(a, Duration::from_millis(2));
        costs.record(a, Duration::from_millis(5));
        costs.record(b, Duration::from_millis(1));
        assert_eq!(
            costs.snapshot(),
            vec![
                (
                    a,
                    TickCost {
                        ticks: 2,
                        total: Duration::from_millis(7),
                        longest: Duration::from_millis(5),
                    }
                ),
                (
                    b,
                    TickCost {
                        ticks: 1,
                        total: Duration::from_millis(1),
                        longest: Duration::from_millis(1),
                    }
                ),
            ]
        );
    }
}
