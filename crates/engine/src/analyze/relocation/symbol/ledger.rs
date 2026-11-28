use std::collections::{BTreeMap, BTreeSet};

use unlace_core::score::ScoreBreakdown;
use unlace_ir::{ContainerId, Node, Polarity};

use crate::analyze::relocation::symbol::inputs::PassInputs;
use crate::analyze::relocation::symbol::outcome::SymbolRelocation;

/// The trial state of one symbol pass: the placement overlay, the running-best
/// objective, the incremental per-file SLOC, occupancy and native-resident
/// counts, and the accepted relocations in acceptance order.
///
/// It answers "where is this node now" and books an accepted move; it never
/// prices, vetoes or nominates anything.
pub(super) struct Ledger<'a> {
    /// Placement of every node under the assembly, the pass's immutable base.
    base: &'a BTreeMap<u32, ContainerId>,
    /// Per-file production SLOC under the assembly, maintained
    /// incrementally across acceptances.
    pub(super) sloc: BTreeMap<ContainerId, u32>,
    /// Per-file production occupancy, maintained alongside [`Self::sloc`].
    pub(super) residents: BTreeMap<ContainerId, u32>,
    /// Per-file count of the production symbols the assembly placed there
    /// that have not yet left — arrivals never raise it (FIX12-A). The
    /// "no empty shells" guard reads this rather than [`Self::residents`],
    /// so an unrelated symbol moving *in* can never license draining the
    /// file's own last resident out.
    pub(super) native: BTreeMap<ContainerId, u32>,
    /// Nodes already relocated in this pass. A symbol moves at most once per
    /// candidate (FIX12-C), so no reader is ever told two contradictory
    /// destinations for the same name.
    pub(super) moved: BTreeSet<u32>,
    /// Effective placement overrides accepted so far.
    pub(super) overlay: BTreeMap<u32, ContainerId>,
    /// Running best objective breakdown; acceptance must beat its `total` by
    /// more than
    /// [`SYMBOL_MIN_IMPROVEMENT`](crate::analyze::relocation::SYMBOL_MIN_IMPROVEMENT).
    pub(super) best: ScoreBreakdown,
    pub(super) relocations: Vec<SymbolRelocation>,
}

impl<'a> Ledger<'a> {
    /// Opens the ledger on the assembly, with `start` as the running best.
    pub(super) fn new(inputs: PassInputs<'a>, start: ScoreBreakdown) -> Self {
        let PassInputs {
            assembled, nodes, ..
        } = inputs;
        let base = &assembled.placement;
        let mut sloc: BTreeMap<ContainerId, u32> = BTreeMap::new();
        let mut residents: BTreeMap<ContainerId, u32> = BTreeMap::new();
        for node in nodes {
            if node.polarity != Polarity::Production {
                continue;
            }
            let Some(file) = base.get(&node.id.0).copied() else {
                continue;
            };
            *sloc.entry(file).or_insert(0) += node.effective_size;
            *residents.entry(file).or_insert(0) += 1;
        }
        let native = residents.clone();
        Self {
            base,
            sloc,
            residents,
            native,
            moved: BTreeSet::new(),
            overlay: BTreeMap::new(),
            best: start,
            relocations: Vec::new(),
        }
    }

    /// Placement of one node: the overlay wins, the assembly fills the rest.
    pub(super) fn effective(&self, id: u32) -> Option<ContainerId> {
        self.overlay
            .get(&id)
            .copied()
            .or_else(|| self.base.get(&id).copied())
    }

    /// Books one accepted relocation: moves the symbol's production SLOC and
    /// resident count from `source` to `destination`, marks the symbol moved
    /// and records it with the breakdowns either side of its acceptance. A
    /// departure lowers the origin's native count; the arrival deliberately
    /// does not raise the destination's (FIX12-A). `fold` is the pass-start id
    /// of the collision-folded file, if any.
    pub(super) fn record_relocation(
        &mut self,
        node: &Node,
        (source, destination): (ContainerId, ContainerId),
        (before, after): (ScoreBreakdown, ScoreBreakdown),
        fold: Option<ContainerId>,
    ) {
        if node.polarity == Polarity::Production {
            self.sloc
                .get_mut(&source)
                .into_iter()
                .for_each(|slot| *slot = slot.saturating_sub(node.effective_size));
            *self.sloc.entry(destination).or_insert(0) += node.effective_size;
            self.residents
                .get_mut(&source)
                .into_iter()
                .for_each(|slot| *slot = slot.saturating_sub(1));
            *self.residents.entry(destination).or_insert(0) += 1;
            self.native
                .get_mut(&source)
                .into_iter()
                .for_each(|slot| *slot = slot.saturating_sub(1));
        }
        self.moved.insert(node.id.0);
        self.relocations.push(SymbolRelocation {
            node: node.id.0,
            from_file: source,
            to_file: destination,
            delta: before.total - after.total,
            before,
            after,
            fold,
        });
    }

    /// Restores the prior overlay entry for one node.
    pub(super) fn undo(&mut self, id: u32, previous: Option<ContainerId>) {
        match previous {
            Some(place) => {
                self.overlay.insert(id, place);
            }
            None => {
                self.overlay.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use unlace_ir::ContainerId;

    use super::Ledger;

    #[test]
    fn should_restore_the_overlay_entry_a_reverted_placement_replaced() {
        let base = BTreeMap::from([(0, ContainerId(4)), (1, ContainerId(4))]);
        let mut ledger = Ledger {
            base: &base,
            sloc: BTreeMap::new(),
            residents: BTreeMap::new(),
            native: BTreeMap::new(),
            moved: BTreeSet::new(),
            overlay: BTreeMap::from([(0, ContainerId(5))]),
            best: unlace_core::score::ScoreBreakdown {
                cut: 0.0,
                imbalance: 0.0,
                naming: 0.0,
                path: 0.0,
                anchor: 0.0,
                capacity: 0.0,
                dependency_only: 0.0,
                companion_separation: 0.0,
                entry_inflow: 0.0,
                unit_outflow: 0.0,
                total: 0.0,
            },
            relocations: Vec::new(),
        };

        let replaced = ledger.overlay.insert(0, ContainerId(6));
        ledger.undo(0, replaced);
        let fresh = ledger.overlay.insert(1, ContainerId(6));
        ledger.undo(1, fresh);

        assert_eq!(ledger.effective(0), Some(ContainerId(5)));
        assert_eq!(ledger.effective(1), Some(ContainerId(4)));
    }
}
