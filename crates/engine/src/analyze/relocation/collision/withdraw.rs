//! Withdrawal of one colliding file move (ADR-21).

use unlace_core::cluster::{ClusterId, Partition};

use crate::analyze::relocation::PipelineSolver;

impl PipelineSolver<'_> {
    /// Withdraws one colliding move (see [`Self::fold_path_collisions`]);
    /// `before` is the round's partition. Returns the newcomers evicted, each
    /// with the cluster it left.
    pub(super) fn withdraw(
        &self,
        before: &Partition,
        parts: &mut Partition,
        mover: usize,
        occupant: usize,
        offered: bool,
    ) -> Vec<(u32, ClusterId)> {
        // every file vertex has an SCC, and every SCC a cluster in both
        // partitions, so the lookups below only keep the withdrawal total.
        let scc_of = |vertex: usize| self.condensation.membership.get(vertex).map(|scc| scc.0);
        let located = scc_of(mover)
            .zip(scc_of(occupant))
            .and_then(|(scc, occupant_scc)| {
                self.pass_start_partition
                    .cluster_of(scc)
                    .zip(before.cluster_of(scc))
                    .zip(before.cluster_of(occupant_scc))
                    .map(|((home, current), landing)| (scc, home, current, landing))
            });
        match located {
            Some((scc, home, current, _)) if current != home => {
                let _moved = parts.move_node(scc, home);
                Vec::new()
            }
            Some((_, home, _, landing)) if offered => {
                self.evict_newcomers(before, parts, home, landing)
            }
            _ => Vec::new(),
        }
    }
}
